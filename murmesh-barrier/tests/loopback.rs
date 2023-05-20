use std::{iter::repeat_with, net::SocketAddr};

use murmesh_barrier::{Message, Service};
use murmesh_bincode::{de, ser};
use murmesh_core::{
    actor::{Drive, State, Wire},
    app::{Closure, PureState},
    transport, Dispatch,
};

use tokio::spawn;

type UserPayload = u16;

async fn use_barrier_udp(addr: SocketAddr, service: SocketAddr) -> Message<UserPayload> {
    // 1. listen to barrier message
    let message = Wire::<Message<UserPayload>>::default();
    let state = transport::Lift(de())
        .install(Closure::from(|(_, message)| message).install(message.state()));
    let out = murmesh_udp::Out::bind(addr).await;
    let local_message = out.0.local_addr().unwrap().port();
    let mut ingress = murmesh_udp::In::new(&out, state);
    let ingress = spawn(async move { ingress.start().await });

    // 2. tx
    transport::Lift(ser())
        .install(out)
        .update((service, local_message));

    // 3. wait for rx
    let mut message = Drive::from(message).recv().await.unwrap();
    ingress.abort();
    message.sort();
    message
}

async fn provide_barrier_udp(addr: SocketAddr, count: usize) {
    let out = murmesh_udp::Out::bind(addr).await;
    let egress = transport::Lift(ser()).install(out.clone());
    let finished = Wire::default();
    let state = transport::Lift(de()).install(Service::<UserPayload, _, _>::new(
        egress,
        finished.state(),
        count,
    ));
    let mut ingress = murmesh_udp::In::new(&out, state);
    let ingress = spawn(async move { ingress.start().await });
    Drive::from(finished).recv().await.unwrap();
    ingress.abort();
}

#[tokio::test]
async fn sync_udp() {
    const N: usize = 2;

    let service_addr = SocketAddr::from(([0, 0, 0, 0], 60000));
    let service = spawn(provide_barrier_udp(service_addr, N));
    // wait for service up?

    let tasks = Vec::from_iter(
        repeat_with(|| {
            spawn(use_barrier_udp(
                SocketAddr::from(([0, 0, 0, 0], 0)),
                service_addr,
            ))
        })
        .take(N),
    );
    let mut messages = Vec::new();
    for task in tasks {
        messages.push(task.await.unwrap());
    }
    println!("{messages:?}");
    assert_eq!(messages[0], messages[1]);

    service.await.unwrap()
}

async fn use_barrier_tcp(addr: SocketAddr, service: SocketAddr) -> Message<UserPayload> {
    let message = Wire::<Message<UserPayload>>::default();
    let mut connection = murmesh_tcp::Connection::connect(
        addr,
        service,
        transport::Lift(de())
            .install(Closure::from(|(_, message)| message).install(message.state())),
        Wire::default().state(),
    )
    .await;
    let local_message = connection.stream.get_ref().local_addr().unwrap().port();
    let mut dispatch = Dispatch::default();
    dispatch.insert_state(connection.remote_addr, connection.out_state());
    let connection = spawn(async move { connection.start().await });

    transport::Lift(ser())
        .install(Closure::from(From::from).install(dispatch))
        .update((service, local_message));

    let mut message = Drive::from(message).recv().await.unwrap();
    connection.abort();
    message.sort();
    message
}

async fn provide_barrier_tcp(addr: SocketAddr, count: usize) {
    let app_wire = Wire::<(SocketAddr, UserPayload)>::default();
    let finished = Wire::default();
    let disconnected = Wire::default();

    let listener = murmesh_tcp::Listener::bind(addr);
    let mut dispatch = Dispatch::default();
    let mut connections = Vec::new();
    for _ in 0..count {
        let mut connection = listener
            .accept(
                transport::Lift(de::<UserPayload>()).install(app_wire.state()),
                disconnected.state(),
            )
            .await;
        dispatch.insert_state(connection.remote_addr, connection.out_state());
        connections.push(spawn(async move { connection.start().await }));
    }
    let app = Service::new(
        transport::Lift(ser()).install(Closure::from(From::from).install(dispatch)),
        finished.state(),
        count,
    );

    let app = spawn(async move { Drive::from(app_wire).run(app).await });
    Drive::from(finished).recv().await.unwrap();
    for connection in connections {
        connection.await.unwrap()
    }
    app.await.unwrap() //
}

#[tokio::test]
async fn sync_tcp() {
    const N: usize = 2;

    let service_addr = SocketAddr::from(([0, 0, 0, 0], 60000));
    let service = spawn(provide_barrier_tcp(service_addr, N));
    // wait for service up?

    let tasks = Vec::from_iter(
        repeat_with(|| {
            spawn(use_barrier_tcp(
                SocketAddr::from(([0, 0, 0, 0], 0)),
                service_addr,
            ))
        })
        .take(N),
    );
    let mut messages = Vec::new();
    for task in tasks {
        messages.push(task.await.unwrap());
    }
    println!("{messages:?}");
    assert_eq!(messages[0], messages[1]);

    service.await.unwrap()
}
