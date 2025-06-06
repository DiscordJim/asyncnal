use std::{sync::Arc, task::Waker};

use arc_swap::ArcSwapOption;





pub(crate) struct LinkyList<T> {
    head: Link<T>,
    tail: Link<T>
}

impl<T> LinkyList<T> {
    pub fn new() -> Self {
        Self {
            head: Link::none(),
            tail: Link::none()
        }
    }
    pub fn push_front(&self, value: T) {
        // let current = &self.head.load().unwrap();
        // while current.value.load().is_some() {
        //     let val =current.next.load();
        // }
        if self.head.value.load().is_none() {
            self.head.set_value(value);
        } else {
            // Moves the current node out.
            self.head.shell_as_next();
            self.head.set_value(value);
        }
    }
    pub fn pop_front(&self) -> Option<T> {
        if self.head.value.load().is_none() {
            
            None
        } else {
            let value = self.head.value.swap(None).unwrap();
            let va = Arc::into_inner(value)?;

            self.head.to_next();

            Some(va)
        }
    }
}



struct Link<T> {
    next: ArcSwapOption<Link<T>>,
    value: ArcSwapOption<T>
}



impl<T> Link<T> {
    fn none() -> Self {
        Self {
            next: ArcSwapOption::default(),
            value: ArcSwapOption::const_empty()
        }
    }
    fn new(inner: T) -> Self {
        Self {
            next: ArcSwapOption::const_empty(),
            value: ArcSwapOption::new(Some(Arc::new(inner)))
        }
    }
    fn set_value(&self, val: T) {
        self.value.store(Some(Arc::new(val)));
    }
    fn shell(&self) -> Link<T> {
        Self {
            next: ArcSwapOption::new(self.next.swap(None)),
            value: ArcSwapOption::new(self.value.swap(None))
        }
    }
    fn shell_as_next(&self) {
        let shelled = Some(Arc::new(self.shell()));
        self.next.store(shelled);

    }
    fn to_next(&self) {
        if self.next.load().is_none() {
            return;
        }
        let loaded = self.next.load();
        let loaded_inner = loaded.as_ref().unwrap();
        self.next.swap(loaded_inner.next.swap(None));
        self.value.swap(loaded_inner.value.swap(None));

    }
}

// Wak

#[cfg(test)]
mod tests {
    use std::task::Waker;

    use crate::linked_list::LinkyList;


    #[test]
    pub fn test_ll_basic() {
        let waka = LinkyList::new();
        waka.push_front(20);
        waka.push_front(12);
        assert_eq!(waka.pop_front(), Some(12));
        assert_eq!(waka.pop_front(), Some(20));
        assert_eq!(waka.pop_front(), None);
        

    }
}