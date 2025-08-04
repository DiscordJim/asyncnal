use std::task::Wake;

pub trait ValidityMarker: Clone + Unpin {
    
    fn set(&self, value: bool);
    fn get(&self) -> bool;
    fn create() -> Self;
}

pub trait TechnicalWaker: Clone {
    fn wake_by_ref(&self);
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

