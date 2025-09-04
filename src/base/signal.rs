
pub trait ValidityMarker: Clone + Unpin {
    
    fn set(&self, value: ValidityState);
    fn get(&self) -> ValidityState;
    fn create() -> Self;
}

#[repr(u8)]
#[derive(Default, PartialEq, Eq, Clone, Copy)]
pub enum ValidityState {
    #[default]
    Idle,
    Killed,
    ShouldWake
}

impl ValidityState {
    pub const fn from_u8(code: u8) -> Option<ValidityState> {
        match code {
            0 => Some(Self::Idle),
            1 => Some(Self::Killed),
            2 => Some(Self::ShouldWake),
            _ => None
        }
    }
}

pub trait TechnicalCounter: Default {
    fn increment(&self);
    fn decrement(&self);
    fn get(&self) -> usize;
}

pub trait WakeQueue<T>: Default {
    fn enqueue(&self, item: T);
    fn dequeue(&self) -> Option<T>;
}

