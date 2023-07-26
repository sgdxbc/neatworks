use std::net::SocketAddr;

use neat_bincode::{de, ser};
use neat_core::{
    app::{Closure, FunctionalState},
    barrier::{Message, Service},
    message::TransportLift,
    wire::WireState,
    Dispatch, Lift, {Drive, State, Wire},
};
use serde::{de::DeserializeOwned, Serialize};
use tokio::spawn;

pub async fn use_barrier<M>(addr: SocketAddr, service: SocketAddr, payload: M) -> Message<M>
where
    M: Serialize + DeserializeOwned + Send + 'static,
{
    let message_wire = Wire::default();
    let egress_wire = Wire::default();
    let mut connection = crate::tcp::Connection::connect(addr, service).await;
    let mut dispatch = Dispatch::default();
    dispatch.insert_state(connection.remote_addr, egress_wire.state());
    let state = message_wire.state();
    let connection = spawn(async move {
        connection
            .start(
                Drive::from(egress_wire),
                de().lift_default::<TransportLift>()
                    .install(Closure(|(_, message)| message).install(state)),
                Wire::default().state(),
            )
            .await
    });

    ser()
        .lift_default::<TransportLift>()
        .install(Closure(From::from).install(dispatch))
        .update((service, payload));

    let message = Drive::from(message_wire).recv().await.unwrap();
    connection.abort();
    message
}

pub async fn provide_barrier<M>(addr: SocketAddr, count: usize)
where
    M: Clone + Serialize + DeserializeOwned + Send + 'static,
{
    let app_wire = Wire::default();
    let finished = Wire::default();

    let listener = crate::tcp::Listener::bind(addr);
    let mut dispatch = Dispatch::default();
    let mut connections = Vec::new();
    for _ in 0..count {
        let egress_wire = Wire::default();
        let mut connection = listener.accept().await;
        dispatch.insert_state(connection.remote_addr, egress_wire.state());
        let state = app_wire.state();
        connections.push(spawn(async move {
            connection
                .start(
                    Drive::from(egress_wire),
                    Lift(de::<M>(), TransportLift).install(state),
                    WireState::dangling(),
                )
                .await
        }));
    }
    let app = Service::new(
        Lift(ser(), TransportLift).install(Closure(From::from).install(dispatch)),
        finished.state(),
        count,
    );

    let app = spawn(async move { Drive::from(app_wire).run(app).await });
    Drive::from(finished).recv().await.unwrap();
    for connection in connections {
        connection.await.unwrap()
    }
    app.await.unwrap()
}
