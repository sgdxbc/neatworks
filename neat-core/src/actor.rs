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

impl<M, T: State<M> + ?Sized, U: std::ops::DerefMut<Target = T>> State<M> for U {
    fn update(&mut self, message: M) {
        T::update(self, message)
    }
}

// want to add blanket impl for `M` as well, but that causes confliction :|

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
