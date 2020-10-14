pub mod http1;
pub mod redis;

use async_trait::async_trait;
use std::{convert::TryFrom, net::TcpListener as StdTcpListener, sync::Arc, thread, task};
use std::{collections::HashMap, net::SocketAddr};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{Mutex, RwLock},
};
use tuzi_proxy::{
    bound::{run_with_listener, ServerAddrProvidable},
    error::TuziResult,
};
use futures::Future;
use hyper::{Body, Client, Uri};
use once_cell::sync::OnceCell;
use tuzi_proxy::{init_tracing, Protocol};
use std::task::Poll;
use hyper::client::{
    connect::{
        dns::{GaiResolver, Resolve},
        http::{ConnectError, HttpConnecting},
    },
    HttpConnector,
};
use std::marker::PhantomData;
use tokio::task::JoinHandle;
use tokio::sync::broadcast;
use tokio::sync::oneshot;
use std::sync::atomic::AtomicPtr;
use std::ptr::null_mut;
use std::sync::atomic::Ordering;
use tokio::task::spawn_blocking;
use testcontainers::{clients, Docker, images};

// static EXIT_SIGNAL_RX: OnceCell<oneshot::Receiver<()>> = OnceCell::new();
static SHUTDOWN_SIGNAL_TX: OnceCell<broadcast::Sender<()>> = OnceCell::new();
static JOIN_HANDLES_PTR: AtomicPtr<Vec<JoinHandle<()>>> = AtomicPtr::new(null_mut());

pub async fn setup_servers() -> &'static Context {
    static CONTEXT: OnceCell<Context> = OnceCell::new();
    let context = CONTEXT.get_or_init(Default::default);

    let b = call_once(async {
        dbg!("call_once");

        init_tracing();

        let mut join_handles = Vec::new();

        // let (exit_signal_tx, exit_signal_rx) = oneshot::channel();
        // EXIT_SIGNAL_RX.set(exit_signal_rx).unwrap();

        let (shutdown_signal_tx, mut shutdown_signal_rx) = broadcast::channel(16);
        SHUTDOWN_SIGNAL_TX.set(shutdown_signal_tx).unwrap();

        // proxy server
        let proxy_listener = TcpListener::bind("localhost:0").await.unwrap();
        context
            .protocol_mapping
            .insert("proxy", proxy_listener.local_addr().unwrap())
            .await;
        join_handles.push(tokio::spawn(run_with_listener(
            proxy_listener,
            context.addr_mapping.clone(),
        )));

        // http server
        let http_listener = StdTcpListener::bind("localhost:0").unwrap();
        context
            .protocol_mapping
            .insert("http1", http_listener.local_addr().unwrap())
            .await;
        join_handles.push(tokio::spawn(http1::server(http_listener, async move {
            shutdown_signal_rx.recv().await.unwrap();
        })));

        // join_handles.push(spawn_blocking(|| {
        //     let docker = clients::Cli::default();
        //     let node = docker.run(images::redis::Redis::default());
        //     let host_port = node.get_host_port(6379).unwrap();
        //     let url = format!("redis://localhost:{}", host_port);
        //     dbg!(url);
        // }));

        JOIN_HANDLES_PTR.store(Box::into_raw(Box::new(join_handles)), Ordering::SeqCst);

        // // wait all join handles
        // tokio::spawn(async move {
        //     for join_handle in join_handles {
        //         join_handle.await.unwrap();
        //     }
        //     exit_signal_tx.send(()).unwrap();
        // });
    })
    .await;

    if !b {
        panic!("run servers failed");
    }

    context
}

#[tokio::test]
async fn teardown() {
    // SHUTDOWN_SIGNAL_TX.get().unwrap().send(()).unwrap();
    // EXIT_SIGNAL_RX.get().unwrap().await;

    // let join_handles = unsafe { Box::from_raw(JOIN_HANDLES_PTR.load(Ordering::SeqCst)) };
    // for join_handle in join_handles.into_iter() {
    //     join_handle.await.unwrap();
    // }
}

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

#[derive(Default)]
pub struct Context {
    pub protocol_mapping: ProtocolServerAddrMapping,
    pub addr_mapping: RealClientServerAddrMapping,
}

impl Context {
    pub async fn add_protocol_server_port<'a>(&'a self, protocol: Protocol, client: &'a TcpStream) {
        let server_addr = self.protocol_mapping.get(protocol).await;
        self.addr_mapping
            .insert(client.local_addr().unwrap(), server_addr)
            .await;
    }

    pub fn build_client(&'static self) -> Client<MockHttpConnector, Body> {
        let connector = MockHttpConnector::new(self);
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
    pub context: &'static Context,
    connector: HttpConnector<R>,
}

impl MockHttpConnector {
    pub fn new(context: &'static Context) -> Self {
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
                    self_.context.add_protocol_server_port("http1", sock).await;
                }
                sock
            }),
            _marker: PhantomData,
        }
    }
}
