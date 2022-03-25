use crate::modifier::StatModifier;
use core::mem::MaybeUninit;

// By default (single-threaded) implementation is most optimized by using std::rc
// if one needs Stat to live in a multithreaded environment, enabling sync feature uses std::sync instead
#[cfg(not(feature = "sync"))]
type ReferenceCounted<T> = std::rc::Rc<T>;
#[cfg(not(feature = "sync"))]
type Weak<T> = std::rc::Weak<T>;
#[cfg(feature = "sync")]
type ReferenceCounted<T> = std::sync::Arc<T>;
#[cfg(feature = "sync")]
type Weak<T> = std::sync::Weak<T>;

/// This handle is returned from calling ```stat.add_modifier()``` (technically it's returned in the Ok, result).
///
/// The handle controls the validity of a modifier.
/// Once dropped, the modifier is automatically removed from the [`super::Stat`] that created it.
pub type StatModifierHandle = ReferenceCounted<StatModifierHandleTag>;

/// Just an empty 'flavor' struct, to indicate that the [`StatModifierHandle`] is an owner of some value
pub struct StatModifierHandleTag;

/// A value that can be modified through [`super::StatModifier`]
///
/// ```const M: usize``` decides how many modifiers a stat can maximally hold (modifier are internally an array on the stack)
pub struct Stat<const M: usize> {
    pub base_value: f32,
    // calculated from base_value and modifiers
    value: f32,
    modifiers: [Option<ModifierMeta>; M],
}

struct ModifierMeta {
    modifier: StatModifier,
    order: i32,
    owner_modifier_weak: Weak<StatModifierHandleTag>,
}

/// This stat can't hold any more modifiers.
/// The [`Stat`] M size should be carefully selected. [`Stat<3>`] [`Stat<7>`]
#[derive(Debug, Clone, Copy)]
pub struct ModifiersFullError;

impl<const M: usize> Stat<M> {
    /// ```
    /// // EXAMPLE: Creates a stat that can hold a maximum of 3 modifiers
    /// # use game_stat::prelude::*;
    /// let attack_stat: Stat<3> = Stat::new(0.0);
    /// let attack_stat = Stat::<3>::new(0.0);
    /// ```
    pub fn new(base_value: f32) -> Self {
        // DANGER DANGER! WARNING WARNING!
        let mut modifiers: [MaybeUninit<Option<ModifierMeta>>; M] =
            unsafe { MaybeUninit::uninit().assume_init() };
        modifiers[..].iter_mut().for_each(|elem| {
            elem.write(None);
        });
        let modifiers = unsafe {
            modifiers
                .as_ptr()
                .cast::<[Option<ModifierMeta>; M]>()
                .read()
        };
        // hopefully we survived that :D
        Self {
            base_value,
            value: base_value,
            modifiers,
        }
    }

    /// Add a modifier using the default order. [`super::StatModifier::default_order()`]
    pub fn add_modifier(
        &mut self,
        modifier: StatModifier,
    ) -> Result<StatModifierHandle, ModifiersFullError> {
        // We have to update the modifiers array in case one has been dropped.
        // The modifier array could be full of data, yet have modifiers that aren't valid.
        // If we drop a modifier and then add one right away, there should be space for it to be added.
        // This ensures the array is up to date.
        self.update_modifiers();
        match self.modifiers.iter_mut().find(|m| m.is_none()) {
            Some(modifier_option) => {
                let key = ReferenceCounted::new(StatModifierHandleTag);
                *modifier_option = Some(ModifierMeta {
                    order: modifier.default_order(),
                    modifier,
                    owner_modifier_weak: ReferenceCounted::downgrade(&key),
                });
                // value needs to update
                self.calculate_value();
                Ok(key)
            }
            None => Err(ModifiersFullError),
        }
    }

    pub fn add_modifier_with_order(
        &mut self,
        modifier: StatModifier,
        order: i32,
    ) -> Result<StatModifierHandle, ModifiersFullError> {
        // We have to update the modifiers array in case one has been dropped.
        // The modifier array could be full of data, yet have modifiers that aren't valid.
        // If we drop a modifier and then add one right away, there should be space for it to be added.
        // This ensures the array is up to date.
        self.update_modifiers();
        match self.modifiers.iter_mut().find(|m| m.is_none()) {
            Some(modifier_option) => {
                let key = ReferenceCounted::new(StatModifierHandleTag);
                *modifier_option = Some(ModifierMeta {
                    modifier,
                    owner_modifier_weak: ReferenceCounted::downgrade(&key),
                    order,
                });
                // value needs to update
                self.calculate_value();
                Ok(key)
            }
            None => Err(ModifiersFullError),
        }
    }

    // check if any modifiers have been dropped, and update the value + array
    fn update_modifiers(&mut self) {
        let any_modifier_dropped = self
            .modifiers
            .iter()
            .filter_map(|m| m.as_ref())
            .any(|m| m.owner_modifier_weak.upgrade().is_none());
        if any_modifier_dropped {
            self.calculate_value();
        }
    }

    /// Returns the base_value with modifiers applied
    pub fn value(&mut self) -> f32 {
        self.update_modifiers();
        self.value
    }

    fn calculate_value(&mut self) {
        let mut value = self.base_value;

        // Order the modifiers
        use std::cmp::Ordering;
        self.modifiers.sort_by(|m1_option, m2_option| {
            if let Some(m1) = m1_option {
                if let Some(m2) = m2_option {
                    m1.order.cmp(&m2.order)
                } else {
                    Ordering::Less
                }
            } else {
                Ordering::Greater
            }
        });
        // hide CLIPPY: we modify the upper nested value, *modifier_meta_option = None
        #[allow(clippy::manual_flatten)]
        for modifier_meta_option in self.modifiers.iter_mut() {
            if let Some(modifier_meta) = modifier_meta_option {
                match modifier_meta.owner_modifier_weak.upgrade() {
                    Some(_key) => modifier_meta.modifier.apply(&mut value),
                    // owner has dropped the modifier, make this modifier available again
                    None => *modifier_meta_option = None,
                }
            }
        }
        self.value = value;
    }
}
