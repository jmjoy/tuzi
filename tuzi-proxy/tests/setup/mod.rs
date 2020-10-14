pub mod http1;

use async_trait::async_trait;
use std::{convert::TryFrom, net::TcpListener as StdTcpListener, sync::Arc, task};

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

pub async fn setup_servers() -> &'static Context {
    static CONTEXT: OnceCell<Context> = OnceCell::new();
    let context = CONTEXT.get_or_init(Default::default);

    let b = call_once(async {
        init_tracing();

        // proxy server
        let proxy_listener = TcpListener::bind("localhost:0").await.unwrap();
        context
            .protocol_mapping
            .insert("proxy", proxy_listener.local_addr().unwrap())
            .await;
        tokio::spawn(run_with_listener(
            proxy_listener,
            context.addr_mapping.clone(),
        ));

        // http server
        let http_listener = StdTcpListener::bind("localhost:0").unwrap();
        context
            .protocol_mapping
            .insert("http1", http_listener.local_addr().unwrap())
            .await;
        tokio::spawn(http1::server(http_listener, None));
    })
    .await;

    if !b {
        panic!("run servers failed");
    }

    context
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
