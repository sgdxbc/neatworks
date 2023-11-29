use axum::Router;

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> helloween::Result<()> {
    std::env::set_var("RUST_BACKTRACE", "1");
    let port = std::env::args()
        .nth(1)
        .as_deref()
        .unwrap_or("10000")
        .parse::<u16>()?;
    let app = AppState::default();
    let shutdown = app.shutdown.clone();
    let app = Router::new();
    let signal_task = tokio::spawn({
        let shutdown = shutdown.clone();
        async move {
            let result = tokio::select! {
                result = tokio::signal::ctrl_c() => result,
                result = shutdown.cancelled() => Ok(result),
            };
            shutdown.cancel();
            result
        }
    });
    // we lose graceful shutdown (in a easy way) after upgrading axum to 0.7, so tentatively give up
    // on that
    // also not sure why only type checks with extra async wrapper
    let serve = async { axum::serve(TcpListener::bind(("0.0.0.0", port)).await?, app).await };
    tokio::select! {
        result = serve => result?,
        result = signal_task => result??,
    }
    Ok(())
}

#[derive(Default)]
struct AppState {
    shutdown: CancellationToken,
}

// type App = State<Arc<AppState>>;
