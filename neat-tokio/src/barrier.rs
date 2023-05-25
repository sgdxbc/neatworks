use std::net::SocketAddr;

use neat_bincode::{de, ser};
use neat_core::{
    app::{Closure, FunctionalState},
    barrier::{Message, Service},
    message::TransportLift,
    Dispatch, Lift, {Drive, State, Wire},
};
use serde::{de::DeserializeOwned, Serialize};
use tokio::spawn;

pub async fn use_barrier<M>(addr: SocketAddr, service: SocketAddr, payload: M) -> Message<M>
where
    M: Serialize + DeserializeOwned + Send + 'static,
{
    let message_wire = Wire::default();
    let mut connection = crate::tcp::Connection::connect(
        addr,
        service,
        de().lift_default::<TransportLift>()
            .install(Closure(|(_, message)| message).install(message_wire.state())),
        Wire::default().state(),
    )
    .await;
    let mut dispatch = Dispatch::default();
    dispatch.insert_state(connection.remote_addr, connection.out_state());
    let connection = spawn(async move { connection.start().await });

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
    let disconnected = Wire::default();

    let listener = crate::tcp::Listener::bind(addr);
    let mut dispatch = Dispatch::default();
    let mut connections = Vec::new();
    for _ in 0..count {
        let mut connection = listener
            .accept(
                Lift(de::<M>(), TransportLift).install(app_wire.state()),
                disconnected.state(),
            )
            .await;
        dispatch.insert_state(connection.remote_addr, connection.out_state());
        connections.push(spawn(async move { connection.start().await }));
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
