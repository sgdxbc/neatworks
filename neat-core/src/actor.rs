use std::ops::{Deref, DerefMut};

pub trait State<Message> {
    fn update(&mut self, message: Message);

    fn boxed(self) -> Box<dyn State<Message>>
    where
        Self: Sized + 'static,
    {
        Box::new(self)
    }

    fn filtered(self) -> Filtered<Self>
    where
        Self: Sized,
    {
        Filtered(self)
    }
}

// want to blanket impl over many `impl DerefMut<Target = T> and `M`, but those
// cause confliction :|
impl<M, T: State<M> + ?Sized> State<M> for &mut T {
    fn update(&mut self, message: M) {
        T::update(self, message)
    }
}

pub trait SharedClone: Clone {}

impl<T: SharedClone> SharedClone for &T {}

impl<T: SharedClone> SharedClone for Box<T> {}

impl<T> SharedClone for std::rc::Rc<T> {}

impl<T> SharedClone for std::sync::Arc<T> {}

pub struct Filtered<S>(pub S);

// TODO can (or is it necessary to) generalize `Option<_>` into `impl Into<_>`?
impl<M, S> State<Option<M>> for Filtered<S>
where
    S: State<M>,
{
    fn update(&mut self, message: Option<M>) {
        if let Some(message) = message {
            self.0.update(message)
        }
    }
}

impl<S> Deref for Filtered<S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<S> DerefMut for Filtered<S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
