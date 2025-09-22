use core::sync::atomic::{AtomicBool, Ordering};

use alloc::sync::Arc;

use crate::{
    arch::with_interrupts,
    process::current::kernel_thread_spawn,
    sockets::{
        SockID, SocketDomain, SocketError, SocketKind, bind_abstract_socket, create_socket,
        get_abstract_binding, get_client_socket,
    },
    thread::{self, Tid},
    utils::{locks::Mutex, types::Name},
};

#[allow(unused)]
fn ipc_stream_test_inner() {
    static BINDED_SOCKET: Mutex<SockID> = Mutex::new(0);
    static SOCKET_DROPPED: AtomicBool = AtomicBool::new(false);

    static CLIENT_MSG: &[u8] = b"Hello from the other side!";
    static SERVER_MSG: &[u8] = b"Your message was received!";

    fn test_thread(_: Tid, (): &()) -> ! {
        let name: Name =
            Name::try_from("safa_core::sockets::test_socket").expect("test socket name too long");

        let sock_id = get_abstract_binding(&name).expect("socket was not binded");
        assert_eq!(*BINDED_SOCKET.lock(), sock_id);
        let sock = get_client_socket(sock_id).expect("socket binding not associated with any id");

        let connection = sock
            .connect(true)
            .expect("socket dropped while waiting for connection");

        let len = connection.write(CLIENT_MSG).expect("client write failed");
        assert_eq!(len, CLIENT_MSG.len());

        let mut data_buf = [0u8; SERVER_MSG.len()];
        connection
            .read(&mut data_buf[..])
            .expect("client read failed");

        assert_eq!(&data_buf[..], SERVER_MSG, "the server's message is wrong");
        drop(connection);

        drop(sock);
        SOCKET_DROPPED.store(true, core::sync::atomic::Ordering::Release);
        thread::current::exit(0);
    }

    let name: Name =
        Name::try_from("safa_core::sockets::test_socket").expect("test socket name too long");

    let sock_desc = create_socket(SocketDomain::Unix, SocketKind::Stream, true);
    let weak_sock = Arc::downgrade(&sock_desc.reference);

    *BINDED_SOCKET.lock() = sock_desc.id;
    bind_abstract_socket(name, sock_desc.id);
    sock_desc.configure_listen_queue(1);

    // Spawn a second thread
    kernel_thread_spawn(test_thread, &(), None, None)
        .expect("failed to spawn the client thread for socket");

    let connection = sock_desc.accept().expect("socket blocked");
    let mut data_buf = [0u8; CLIENT_MSG.len()];

    connection
        .read(&mut data_buf[..])
        .expect("server read failed");
    assert_eq!(&data_buf[..], CLIENT_MSG, "The client's message is wrong");

    connection.write(SERVER_MSG).expect("server write failed");
    assert_eq!(
        connection.read(&mut []),
        Err(SocketError::ConnectionClosed),
        "Read didn't fail with connection closed, even after it was"
    );
    assert_eq!(
        connection.write(&[]),
        Err(SocketError::ConnectionClosed),
        "Write didn't fail with connection closed, even after it was"
    );

    drop(sock_desc);
    drop(connection);

    while !SOCKET_DROPPED.load(Ordering::Acquire) {}
    assert!(
        weak_sock.strong_count() == 0,
        "The socket has a tailing reference, got {} references",
        weak_sock.strong_count()
    );
}

#[allow(unused)]
fn ipc_seqpacket_test_inner() {
    let name: Name =
        Name::try_from("safa_core::sockets::test_socket").expect("test socket name too long");

    static CLIENT_MSG0: &[u8] = b"Hello from the other side!";
    static CLIENT_MSG1: &[u8] = b"Reply if you received this message!";
    static SERVER_MSG: &[u8] = b"Your message was received!";
    static THREAD_EXIT: AtomicBool = AtomicBool::new(false);

    fn test_thread(_: Tid, (): &()) -> ! {
        {
            let name: Name = Name::try_from("safa_core::sockets::test_socket")
                .expect("test socket name too long");

            let sock_id = get_abstract_binding(&name).expect("Socket was not binded");
            let sock =
                get_client_socket(sock_id).expect("Socket binding not associated with any id");

            let connection = sock
                .connect(true)
                .expect("Socket dropped while waiting for connection");

            let len = connection
                .write(CLIENT_MSG0)
                .expect("Client failed to write the first message");
            assert_eq!(len, CLIENT_MSG0.len());

            let len = connection
                .write(CLIENT_MSG1)
                .expect("Client failed to write the second message");
            assert_eq!(len, CLIENT_MSG1.len());

            let mut read_buf = [0u8; SERVER_MSG.len()];
            connection
                .read(&mut read_buf[..])
                .expect("Client failed to read the server's message");

            assert_eq!(&read_buf[..], SERVER_MSG, "Server's message was wrong");
        }
        THREAD_EXIT.store(true, Ordering::Release);
        thread::current::exit(0);
    }

    let sock_desc = create_socket(SocketDomain::Unix, SocketKind::SeqPacket, true);
    bind_abstract_socket(name, sock_desc.id);
    sock_desc.configure_listen_queue(1);

    // Spawn a second thread
    kernel_thread_spawn(test_thread, &(), None, None)
        .expect("failed to spawn the client thread for socket");

    let accepted = sock_desc.accept().expect("Socket is non-blocking?");
    // If we were a stream, we should have ended up reading both
    let mut msg_buf = [0u8; CLIENT_MSG0.len() + CLIENT_MSG1.len()];

    let read = accepted
        .read(&mut msg_buf)
        .expect("Server failed to read the first message");
    let msg0 = &msg_buf[..read];
    assert_eq!(msg0, CLIENT_MSG0, "Client's first message mismatch");

    let read = accepted
        .read(&mut msg_buf)
        .expect("Server failed to read the second message");
    let msg1 = &msg_buf[..read];
    assert_eq!(msg1, CLIENT_MSG1, "Client's second message mismatch");

    accepted
        .write(SERVER_MSG)
        .expect("Server failed to write a message");

    while !THREAD_EXIT.load(Ordering::Acquire) {}

    assert_eq!(
        accepted.write(SERVER_MSG),
        Err(SocketError::ConnectionClosed),
        "Write was successful even though connection should have been closed"
    );
    assert_eq!(
        accepted.read(&mut msg_buf),
        Err(SocketError::ConnectionClosed),
        "Read was successful even though connection should have been closed"
    );
}

// TODO: Tests are ordered alphatically so we want to run this last, in my framework some modules do have a priority over other tho so I could use that
#[test_case]
fn z_ipc0_stream() {
    with_interrupts(|| ipc_stream_test_inner());
}

#[test_case]
fn z_ipc1_seqpacket() {
    with_interrupts(|| ipc_seqpacket_test_inner());
}
