pub mod http1;
pub mod redis;
pub mod unknown;

use async_trait::async_trait;
use futures::{ready, Future, FutureExt, TryFutureExt};
use hyper::{
    client::{
        connect::{
            dns::{GaiResolver, Resolve},
            http::{ConnectError, HttpConnecting},
        },
        HttpConnector,
    },
    Body, Client, Uri,
};
use once_cell::sync::OnceCell;
use std::{
    collections::HashMap,
    convert::TryFrom,
    marker::PhantomData,
    net::{SocketAddr, TcpListener as StdTcpListener},
    pin::Pin,
    ptr::null_mut,
    sync::{
        atomic::{AtomicPtr, Ordering},
        Arc,
    },
    task,
    task::Poll,
    thread,
};
use testcontainers::{clients, images, Docker};
use tokio::{
    net::{TcpListener, TcpStream},
    runtime::Handle,
    sync::{broadcast, oneshot, Mutex, RwLock},
    task::{spawn_blocking, JoinHandle},
};
use tuzi_proxy::{
    bound::{run_with_listener, ServerAddrProvidable},
    error::TuziResult,
    init_tracing,
    wait::WaitGroup,
    Protocol,
};

pub async fn setup() -> (Context, Handles) {
    init_tracing();

    let mut context: Context = Default::default();
    let (mut handles, mut shutdown_signal_rx) = Handles::new();
    let wg = WaitGroup::new();

    // proxy server
    let proxy_listener = TcpListener::bind("localhost:0").await.unwrap();
    context
        .protocol_mapping
        .insert("proxy", proxy_listener.local_addr().unwrap())
        .await;
    handles.server_handles.push(tokio::spawn(run_with_listener(
        proxy_listener,
        context.addr_mapping.clone(),
        recv_signal(shutdown_signal_rx).map(Ok),
    )));

    // http server
    let http_listener = StdTcpListener::bind("localhost:0").unwrap();
    context
        .protocol_mapping
        .insert("http1", http_listener.local_addr().unwrap())
        .await;
    handles.server_handles.push(tokio::spawn(http1::server(
        http_listener,
        recv_signal(handles.shutdown_signal_tx.subscribe()),
    )));

    // redis server
    handles.server_handles.push(tokio::spawn(redis::server(
        wg.clone(),
        recv_signal(handles.shutdown_signal_tx.subscribe()),
        context.clone().protocol_server_port_inserter("redis"),
    )));

    // unknown server
    handles.server_handles.push(tokio::spawn(unknown::server(
        recv_signal(handles.shutdown_signal_tx.subscribe()),
        context.clone().protocol_server_port_inserter("unknown"),
    )));

    // JOIN_HANDLES_PTR.store(Box::into_raw(Box::new(join_handles)), Ordering::SeqCst);

    // // wait all join handles
    // tokio::spawn(async move {
    //     for join_handle in join_handles {
    //         join_handle.await.unwrap();
    //     }
    //     exit_signal_tx.send(()).unwrap();
    // });

    wg.wait().await;

    (context, handles)
}

async fn recv_signal(mut signal: broadcast::Receiver<()>) {
    signal.recv().await.unwrap();
}

// async fn teardown() {
// SHUTDOWN_SIGNAL_TX.get().unwrap().send(()).unwrap();
// EXIT_SIGNAL_RX.get().unwrap().await;

// let join_handles = unsafe { Box::from_raw(JOIN_HANDLES_PTR.load(Ordering::SeqCst)) };
// for join_handle in join_handles.into_iter() {
//     join_handle.await.unwrap();
// }
// }

pub async fn call_once(fut: impl Future<Output = ()>) -> bool {
    static START: OnceCell<Mutex<(bool, bool)>> = OnceCell::new();
    let start = START.get_or_init(|| Mutex::new((false, false)));
    let mut lock = start.lock().await;
    if lock.0 {
        return lock.1;
    }
    lock.0 = true;
    fut.await;
    lock.1 = true;
    lock.1
}

pub struct Handles {
    server_handles: Vec<JoinHandle<()>>,
    test_handles: Vec<JoinHandle<()>>,
    shutdown_signal_tx: broadcast::Sender<()>,
}

impl Handles {
    pub fn new() -> (Self, broadcast::Receiver<()>) {
        let (shutdown_signal_tx, shutdown_signal_rx) = broadcast::channel(16);
        (
            Self {
                server_handles: vec![],
                test_handles: vec![],
                shutdown_signal_tx,
            },
            shutdown_signal_rx,
        )
    }

    pub async fn add_test(&mut self, fut: impl Future<Output = ()> + Send + 'static) {
        self.test_handles.push(tokio::spawn(fut));
    }

    pub async fn wait(self) {
        for handle in self.test_handles {
            handle.await.unwrap();
        }

        self.shutdown_signal_tx.send(()).unwrap();

        for handle in self.server_handles {
            handle.await.unwrap();
        }
    }
}

impl Future for Handles {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
        for handle in &mut self.test_handles {
            ready!(Pin::new(handle).poll(cx)).unwrap();
        }

        self.shutdown_signal_tx.send(()).unwrap();

        for handle in &mut self.server_handles {
            ready!(Pin::new(handle).poll(cx)).unwrap();
        }

        Poll::Ready(())
    }
}

#[derive(Default, Clone)]
pub struct Context {
    pub protocol_mapping: ProtocolServerAddrMapping,
    pub addr_mapping: RealClientServerAddrMapping,
}

impl Context {
    pub async fn insert_protocol_client_port<'a>(
        &'a self,
        protocol: Protocol,
        client: &'a TcpStream,
    ) {
        let server_addr = self.protocol_mapping.get(protocol).await;
        self.addr_mapping
            .insert(client.local_addr().unwrap(), server_addr)
            .await;
    }

    pub async fn insert_protocol_server_port<'a>(
        &'a self,
        protocol: Protocol,
        server: &'a TcpStream,
    ) {
        self.protocol_mapping
            .insert(protocol, server.local_addr().unwrap())
            .await;
    }

    fn protocol_server_port_inserter(
        self,
        protocol: Protocol,
    ) -> impl FnOnce(SocketAddr) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        move |addr| {
            Box::pin(async move {
                self.protocol_mapping.insert(protocol, addr).await;
            })
        }
    }

    pub fn build_client(&self) -> Client<MockHttpConnector, Body> {
        let connector = MockHttpConnector::new(self.clone());
        let client = Client::builder().build::<_, Body>(connector);
        client
    }

    pub async fn build_http_uri(&self, s: impl AsRef<str>) -> Uri {
        let uri = format!(
            "http://{}{}",
            self.protocol_mapping.get("proxy").await,
            s.as_ref()
        );
        let uri = Uri::try_from(uri).unwrap();
        uri
    }
}

#[derive(Default, Clone)]
pub struct ProtocolServerAddrMapping {
    inner: Arc<RwLock<HashMap<Protocol, SocketAddr>>>,
}

impl ProtocolServerAddrMapping {
    pub async fn get(&self, protocol: Protocol) -> SocketAddr {
        let lock = self.inner.read().await;
        let addr = lock.get(protocol).unwrap();
        addr.clone()
    }

    pub async fn insert(&self, protocol: Protocol, server_addr: SocketAddr) {
        let mut lock = self.inner.write().await;
        lock.insert(protocol, server_addr);
    }
}

#[derive(Default, Clone)]
pub struct RealClientServerAddrMapping {
    inner: Arc<RwLock<HashMap<SocketAddr, SocketAddr>>>,
}

impl RealClientServerAddrMapping {
    pub async fn insert(&self, peer: SocketAddr, server_addr: SocketAddr) {
        let mut lock = self.inner.write().await;
        lock.insert(peer, server_addr);
    }
}

#[async_trait]
impl ServerAddrProvidable for RealClientServerAddrMapping {
    async fn provide_server_addr(&self, socket: &TcpStream) -> TuziResult<SocketAddr> {
        let lock = self.inner.read().await;
        let addr = lock.get(&socket.peer_addr().unwrap()).unwrap();
        Ok(addr.clone())
    }
}

#[derive(Clone)]
pub struct MockHttpConnector<R = GaiResolver> {
    pub context: Context,
    connector: HttpConnector<R>,
}

impl MockHttpConnector {
    pub fn new(context: Context) -> Self {
        Self {
            context,
            connector: HttpConnector::new(),
        }
    }
}

impl<R> tower_service::Service<Uri> for MockHttpConnector<R>
where
    R: Resolve + Clone + Send + Sync + 'static,
    R::Future: Send,
{
    type Response = TcpStream;
    type Error = ConnectError;
    type Future = HttpConnecting<R>;

    fn poll_ready(&mut self, cx: &mut task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.connector.poll_ready(cx)
    }

    fn call(&mut self, dst: Uri) -> Self::Future {
        let mut self_ = self.clone();
        HttpConnecting {
            fut: Box::pin(async move {
                let sock = self_.connector.call_async(dst).await;
                if let Ok(sock) = &sock {
                    self_
                        .context
                        .insert_protocol_client_port("http1", sock)
                        .await;
                }
                sock
            }),
            _marker: PhantomData,
        }
    }
}
