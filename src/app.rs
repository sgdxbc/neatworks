use crate::model::{SubmitHandle, SubmitSource};

pub async fn null_session<T, U>(mut source: SubmitSource<T, U>) -> crate::Result<()>
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

pub async fn null_workload_session(
    invoke_handle: SubmitHandle<Vec<u8>, Vec<u8>>,
) -> crate::Result<()> {
    invoke_handle.submit(Default::default()).await?;
    Ok(())
}
