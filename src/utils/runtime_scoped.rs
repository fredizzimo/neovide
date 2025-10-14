use std::{
    option::Option,
    ops::{Deref, DerefMut}
};

#[derive(PartialEq)]
enum RuntimeState {
    NeverInitialized,
    Initialized,
    Destroyed,
}

pub struct RuntimeScoped<T> {
    value: Option<T>,
    state: RuntimeState,
}

impl<T> RuntimeScoped<T> {
    pub fn uninitialized() -> Self {
        Self {
            value: None,
            state: RuntimeState::NeverInitialized,
        }
    }

    pub fn with_value(val: T) -> Self {
        Self {
            value: Some(val),
            state: RuntimeState::Initialized,
        }
    }

    pub fn initialize(&mut self, val: T) {
        match self.state {
            RuntimeState::Initialized => panic!("Already initialized"),
            RuntimeState::Destroyed => panic!("Cannot initialize after destruction"),
            RuntimeState::NeverInitialized => {
                self.value = Some(val);
                self.state = RuntimeState::Initialized;
            }
        }
    }

    pub fn destroy(&mut self) {
        match self.state {
            RuntimeState::Initialized => {
                self.value = None;
                self.state = RuntimeState::Destroyed;
            }
            RuntimeState::NeverInitialized => panic!("Cannot destroy before initialization"),
            RuntimeState::Destroyed => panic!("Already destroyed"),
        }
    }

    pub fn is_valid(&self) -> bool {
        self.state == RuntimeState::Initialized
    }
}

impl<T> Deref for RuntimeScoped<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        match self.state {
            RuntimeState::Initialized => self.value.as_ref().expect("Value should be initialized"),
            RuntimeState::NeverInitialized => panic!("Value never initialized"),
            RuntimeState::Destroyed => panic!("Value destroyed"),
        }
    }
}

impl<T> DerefMut for RuntimeScoped<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self.state {
            RuntimeState::Initialized => self.value.as_mut().expect("Value should be initialized"),
            RuntimeState::NeverInitialized => panic!("Value never initialized"),
            RuntimeState::Destroyed => panic!("Value destroyed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state() {
        let scoped: RuntimeScoped<i32> = RuntimeScoped::uninitialized();
        assert!(!scoped.is_valid());
    }

    #[test]
    fn test_initialize_and_deref() {
        let mut scoped: RuntimeScoped<i32> = RuntimeScoped::uninitialized();
        scoped.initialize(42);
        assert!(scoped.is_valid());
        assert_eq!(*scoped, 42);
    }

    #[test]
    fn test_with_value_constructor() {
        let scoped = RuntimeScoped::with_value(42);
        assert!(scoped.is_valid());
        assert_eq!(*scoped, 42);
    }

    #[test]
    #[should_panic(expected = "Already initialized")]
    fn test_double_initialize_panics() {
        let mut scoped = RuntimeScoped::with_value(1);
        scoped.initialize(2); // Should panic
    }

    #[test]
    fn test_destroy_and_state() {
        let mut scoped = RuntimeScoped::with_value(99);
        scoped.destroy();
        assert!(!scoped.is_valid());
    }

    #[test]
    #[should_panic(expected = "Value destroyed")]
    fn test_deref_after_destroy_panics() {
        let mut scoped = RuntimeScoped::with_value(123);
        scoped.destroy();
        let _ = *scoped; // Should panic
    }

    #[test]
    #[should_panic(expected = "Cannot initialize after destruction")]
    fn test_initialize_after_destroy_panics() {
        let mut scoped = RuntimeScoped::with_value(1);
        scoped.destroy();
        scoped.initialize(2); // Should panic
    }

    #[test]
    #[should_panic(expected = "Cannot destroy before initialization")]
    fn test_destroy_before_initialize_panics() {
        let mut scoped: RuntimeScoped<i32> = RuntimeScoped::uninitialized();
        scoped.destroy(); // Should panic
    }

    #[test]
    #[should_panic(expected = "Already destroyed")]
    fn test_destroy_twice_panics() {
        let mut scoped = RuntimeScoped::with_value(1);
        scoped.destroy();
        scoped.destroy(); // Should panic
    }
}
