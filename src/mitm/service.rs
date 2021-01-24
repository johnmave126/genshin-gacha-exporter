/// A man-in-the-middle proxy to only intercept gacha log page
use std::{
    convert::Infallible,
    future::{self, Future, Ready},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::future::TryFutureExt;
use hyper::{
    client::{
        connect::{dns::GaiResolver, HttpConnector},
        Client,
    },
    server::{
        conn::{AddrIncoming, AddrStream, Http},
        Server,
    },
    service::{service_fn, Service},
    upgrade, Body, Method, Request, Response, Uri,
};
use hyper_rustls::HttpsConnector;
use reqwest::Url;
use rustls::{Certificate, NoClientAuth, PrivateKey, ServerConfig};
use tokio::{
    io::{copy as async_copy, split as async_split},
    net::TcpStream,
    sync::mpsc,
    task::spawn,
};
use tokio_rustls::TlsAcceptor;

use crate::mitm::{DOMAIN_INTERCEPT, PAGE_INTERCEPT_SUFFIX};

#[derive(Clone)]
pub struct MitmService {
    client: Arc<Client<HttpsConnector<HttpConnector<GaiResolver>>, Body>>,
    tls_cfg: Arc<ServerConfig>,
    sender: mpsc::Sender<Url>,
}

impl MitmService {
    fn new(certificate: Certificate, private_key: PrivateKey) -> (mpsc::Receiver<Url>, Self) {
        let (sender, receiver) = mpsc::channel(16);
        let mut tls_cfg = ServerConfig::new(NoClientAuth::new());
        tls_cfg
            .set_single_cert(vec![certificate], private_key)
            .unwrap();
        (
            receiver,
            Self {
                client: Arc::new(Client::builder().build(HttpsConnector::with_native_roots())),
                tls_cfg: Arc::new(tls_cfg),
                sender,
            },
        )
    }
}

/// `MitmService` as `MakeService`
impl Service<&AddrStream> for MitmService {
    type Response = MitmService;
    type Error = Infallible;
    type Future = Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _socket: &AddrStream) -> Self::Future {
        future::ready(Ok(self.clone()))
    }
}

/// `MitmService` as `HttpService`
impl Service<Request<Body>> for MitmService {
    type Response = Response<Body>;
    type Error = anyhow::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        if *req.method() == Method::CONNECT {
            // Handle SSL request
            let host = req.uri().authority().map(|a| a.host());
            if DOMAIN_INTERCEPT.iter().any(|&domain| host == Some(domain)) {
                Box::pin(self.clone().proxy_intercept(req))
            } else {
                Box::pin(self.clone().proxy_pass_tls(req))
            }
        } else {
            Box::pin(self.clone().proxy_pass_http(req))
        }
    }
}

impl MitmService {
    /// Intercept the request, if the following uri match what we are looking for, send it through the channel
    async fn proxy_intercept(self, mut req: Request<Body>) -> anyhow::Result<Response<Body>> {
        spawn(async move {
            let tls_cfg = self.tls_cfg;
            let client = self.client;
            let sender = self.sender;
            if let Ok(stream) = upgrade::on(&mut req)
                .map_err(anyhow::Error::from)
                .and_then(|upgraded: upgrade::Upgraded| {
                    TlsAcceptor::from(tls_cfg)
                        .accept(upgraded)
                        .map_err(anyhow::Error::from)
                })
                .await
            {
                let service = service_fn(move |mut req: Request<Body>| {
                    let client = client.clone();
                    let sender = sender.clone();
                    async move {
                        let new_uri = Uri::builder()
                            .scheme("https")
                            .authority(
                                req.headers()
                                    .get("host")
                                    .map(|h| h.to_str().unwrap())
                                    .unwrap(),
                            )
                            .path_and_query(
                                req.uri()
                                    .path_and_query()
                                    .map(|pq| pq.as_str())
                                    .unwrap_or("/"),
                            )
                            .build()
                            .unwrap();
                        *req.uri_mut() = new_uri;
                        if req
                            .uri()
                            .path_and_query()
                            .map(|pq| pq.path().ends_with(PAGE_INTERCEPT_SUFFIX))
                            == Some(true)
                        {
                            let url = req.uri().to_string().parse().unwrap();
                            sender.send(url).await?;
                        }
                        Ok::<_, anyhow::Error>(client.request(req).await?)
                    }
                });
                let http = Http::new();
                let server = http.serve_connection(stream, service);
                server.await.ok();
            }
        });
        Ok(Response::new(Body::empty()))
    }

    /// Upgrade the connection to TCPStream and pipe it to upstream authority
    async fn proxy_pass_tls(self, mut req: Request<Body>) -> anyhow::Result<Response<Body>> {
        let mut remote_stream = Self::acquire_connection(&req).await?;
        spawn(async move {
            if let Ok(upgraded) = upgrade::on(&mut req).await {
                let (mut remote_read, mut remote_write) = remote_stream.split();
                let (mut client_read, mut client_write) = async_split(upgraded);

                let client_to_remote = async_copy(&mut client_read, &mut remote_write);
                let remote_to_client = async_copy(&mut remote_read, &mut client_write);

                tokio::try_join!(client_to_remote, remote_to_client).ok();
            }
        });
        Ok(Response::new(Body::empty()))
    }

    /// Pass the proxy request along without tapping
    async fn proxy_pass_http(self, req: Request<Body>) -> anyhow::Result<Response<Body>> {
        Ok(self.client.request(req).await?)
    }

    /// Acquire a raw tcp connection to the authority of the request
    async fn acquire_connection(req: &Request<Body>) -> anyhow::Result<TcpStream> {
        let mut connector = HttpConnector::new();
        let uri = hyper::Uri::builder()
            .scheme("http")
            .authority(
                req.uri()
                    .authority()
                    .map(|a| a.as_str())
                    .unwrap_or_default(),
            )
            .path_and_query("/")
            .build()
            .unwrap();
        Ok(connector.call(uri).await?)
    }
}

/// Create a man-in-the-middle proxy server and a receiver to receive the detected url
pub fn make_mitm_server(
    certificate: Certificate,
    private_key: PrivateKey,
) -> (mpsc::Receiver<Url>, Server<AddrIncoming, MitmService>) {
    let (receiver, service) = MitmService::new(certificate, private_key);

    (
        receiver,
        Server::bind(&"0.0.0.0:0".parse().unwrap()).serve(service),
    )
}
