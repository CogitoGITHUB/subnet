#[derive(Debug, PartialEq, Eq)]
pub struct EditStack<T> {
    /// States before the current one, oldest first.
    undo: Vec<T>,
    /// The state currently pointed at. Always exists, so no index bounds to check.
    current: T,
    /// States after the current one, nearest first (a stack).
    redo: Vec<T>,
}

impl<T> EditStack<T> {
    pub fn new() -> Self
    where
        T: Default,
    {
        EditStack {
            undo: Vec::new(),
            current: T::default(),
            redo: Vec::new(),
        }
    }
}

impl<T> EditStack<T>
where
    T: Default,
{
    /// Go back one point in the undo stack. If present on first edit do nothing
    pub(super) fn undo(&mut self) -> &T {
        if let Some(prev) = self.undo.pop() {
            let cur = std::mem::replace(&mut self.current, prev);
            self.redo.push(cur);
        }
        &self.current
    }

    /// Go forward one point in the undo stack. If present on the last edit do nothing
    pub(super) fn redo(&mut self) -> &T {
        if let Some(next) = self.redo.pop() {
            let cur = std::mem::replace(&mut self.current, next);
            self.undo.push(cur);
        }
        &self.current
    }

    /// Insert a new entry to the undo stack.
    /// NOTE: (IMP): If we have hit undo a few times then discard all the other values that come
    /// after the current point
    pub(super) fn insert(&mut self, value: T) {
        self.redo.clear();
        let prev = std::mem::replace(&mut self.current, value);
        self.undo.push(prev);
    }

    /// Reset the stack to the initial state
    pub(super) fn reset(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.current = T::default();
    }

    /// Return the entry currently being pointed to
    pub(super) fn current(&mut self) -> &T {
        &self.current
    }
}

