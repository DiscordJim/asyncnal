use std::{ops::Deref, ptr::{null_mut, NonNull}, sync::{atomic::{AtomicPtr, AtomicU8, Ordering}, Arc}, task::Waker};

use arc_swap::ArcSwapOption;

use crate::atom_lock::AtomicLock;



struct LinkPtr<T>(AtomicPtr<Link<T>>);


impl<T> LinkPtr<T> {
    pub fn new(link: Link<T>) -> Self {
        let alloc = Box::into_raw(Box::new(link));

        Self::from_raw(alloc)
    }
    pub fn from_raw(link_ptr: *mut Link<T>) -> Self {
        Self(AtomicPtr::new(link_ptr))
    }
    pub fn null() -> Self {
        Self(AtomicPtr::default())
    }
    pub fn is_null(&self) -> bool {
        self.load_ptr().is_null()
    }
    fn load_ptr(&self) -> *mut Link<T> {
        self.0.load(Ordering::Acquire)
    }
    fn store_ptr(&self, ptr: *mut Link<T>) {
        self.0.store(ptr, Ordering::Release);
    }
    pub fn equalize(&self, other: &Self) {
        let addy = other.load_ptr();
        self.0.store(addy, Ordering::Release);
    }
}



pub(crate) struct LinkyList<T> {
    head: AtomicLock<LinkPtr<T>>,
    tail: AtomicLock<LinkPtr<T>>
}

impl<T> LinkyList<T> {
    pub fn new() -> Self {
        Self {
            head: AtomicLock::new(LinkPtr::null()),
            tail: AtomicLock::new(LinkPtr::null()),
        }
    }
    pub fn push_front(&self, value: T) {

        let head_lock = self.head.lock();
        let tail_lock = self.tail.lock();

        if head_lock.is_null() {
            // We will need to update both pointers so we will drop the tail lock.
            let ptr = Link::new_no_next(value);
            head_lock.store_ptr(ptr);
            tail_lock.store_ptr(ptr);
        } else {
            drop(tail_lock); // we will not be modifying the tail.
            let new = Link::new_with_next(value, head_lock.load_ptr());
            head_lock.store_ptr(new);
        }
    }
    pub fn pop_front(&self) -> Option<T> {
        let head_lock = self.head.lock();
        

        if head_lock.is_null() {
            None
        } else {
            let tail_lock = self.tail.lock();
            let reclaimed = *unsafe { Box::from_raw(head_lock.load_ptr()) };
            if reclaimed.next.is_null() {
                // In this case, the head is actually also the tail.
                tail_lock.store_ptr(null_mut());
            } else {
                drop(tail_lock); // The tail is safe from modification.
                
            }
            head_lock.store_ptr(reclaimed.next.load_ptr());

            Some(reclaimed.value)
        }


        // if self.head.is_null() {
        //     None
        // } else {
        //     self.lock();
        //     let head_ptr = self.head.load_ptr();
        //     let reclaimed = *unsafe { Box::from_raw(head_ptr) };
        //     if reclaimed.next.is_null() {
        //         println!("ZEROED");
        //         self.tail.0.store(null_mut(), Ordering::Release);
        //     }
        //     self.head.0.store(reclaimed.next.load_ptr(), Ordering::Release);
        //     self.unlock();
        //     Some(reclaimed.value)
        // }
    }
}



struct Link<T> {
    next: LinkPtr<T>,
    value: T
}

impl<T> Link<T> {
    pub fn new_no_next(value: T) -> *mut Self {
        Self::from_parts(value, LinkPtr::null())
    }
    pub fn new_with_next(value: T, raw_next: *mut Link<T>) -> *mut Self {
        Self::from_parts(value, LinkPtr::from_raw(raw_next))
    }
    fn from_parts(value: T, next: LinkPtr<T>) -> *mut Self {
        Box::into_raw(Box::new(Link {
            value,
            next
        }))
    }
}



// Wak

#[cfg(test)]
mod tests {
    use std::{sync::Arc, task::Waker};

    use tokio::sync::Barrier;

    use crate::linked_list::LinkyList;


    #[tokio::test]
    pub async fn test_ll_basic() {
        let waka = LinkyList::new();
        

        waka.push_front(20);
        waka.push_front(12);
        assert_eq!(waka.pop_front(), Some(12));
        assert_eq!(waka.pop_front(), Some(20));
        assert_eq!(waka.pop_front(), None);
        

    }

    #[tokio::test]
    pub async fn test_ll_syncy() {
        let waka = Arc::new(LinkyList::new());

        let threads = 10;

        let arcy = Arc::new(Barrier::new(threads + 1));

        let mut handles = vec![];

        for i in 0..threads {
            handles.push(tokio::spawn({
                let arcy = arcy.clone();
                let waka = waka.clone();
                async move {
                    arcy.wait().await;

                    for i in 0..1000 {
                        waka.push_front(i);
                    }
                    for i in 0..1000 {
                        waka.pop_front();
                    }

                }
            }));
        }


        arcy.wait().await;

        for handle in handles {
            handle.await.unwrap();
        }
        

        

    }


    
}