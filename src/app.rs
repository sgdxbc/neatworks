use crate::model::SubmitSource;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Null;

impl Null {
    pub async fn session<T, U>(self, mut source: SubmitSource<T, U>) -> crate::Result<()>
    where
        U: Default,
    {
        while let Some((_, result)) = source.option_next().await {
            result
                .resolve(Default::default())
                .map_err(|_| crate::err!("unexpected reply channel closing"))?
        }
        Ok(())
    }
}
