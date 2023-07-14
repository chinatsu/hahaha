use futures::Future;
use hyper::service::{make_service_fn, service_fn};
use hyper::{server::Server, Body, Request, Response};
use prometheus::{register_int_counter, register_int_counter_vec, Encoder, IntCounter, IntCounterVec, TextEncoder};
use tracing::{error, info};

lazy_static! {
    pub static ref SIDECAR_SHUTDOWNS: IntCounterVec = register_int_counter_vec!(
        "hahaha_sidecar_shutdowns",
        "Number of sidecar shutdowns",
        &["container", "job_name", "namespace"],
    )
    .unwrap();
    pub static ref FAILED_SIDECAR_SHUTDOWNS: IntCounterVec = register_int_counter_vec!(
        "hahaha_failed_sidecar_shutdowns",
        "Number of failed sidecar shutdowns",
        &["container", "job_name", "namespace"],
    )
    .unwrap();
    pub static ref TOTAL_UNSUCCESSFUL_EVENT_POSTS: IntCounter = register_int_counter!(
        "hahaha_total_unsuccessful_event_posts",
        "Total number of unsuccessful Kubernetes Event posts"
    )
    .unwrap();
    pub static ref UNSUPPORTED_SIDECARS: IntCounterVec = register_int_counter_vec!(
        "hahaha_unsupported_sidecars",
        "Number of unsupported sidecars, by sidecar",
        &["container", "job_name", "namespace"],
    )
    .unwrap();
}

/// The function which triggers on any request to the server (incl. any path)
async fn metric_service(_req: Request<Body>) -> hyper::Result<Response<Body>> {
    let encoder = TextEncoder::new();
    let mut buffer = vec![];
    let mf = prometheus::gather();
    encoder.encode(&mf, &mut buffer).unwrap();
    Ok(Response::builder()
        .header(hyper::header::CONTENT_TYPE, encoder.format_type())
        .body(Body::from(buffer))
        .unwrap())
}

/// The function which spawns the prometheus server
///
/// F is generally a Notify awaiting a notification
pub async fn prometheus_server<F>(port: u16, shutdown: F) -> hyper::Result<()>
where
    F: Future<Output = ()>,
{
    let addr = ([0, 0, 0, 0], port).into();
    info!("serving prometheus on http://{addr}");

    let service = make_service_fn(move |_| async { Ok::<_, hyper::Error>(service_fn(metric_service)) });
    let err = Server::bind(&addr)
        .serve(service)
        .with_graceful_shutdown(shutdown)
        .await;
    match &err {
        Ok(()) => info!("stopped prometheus server successfully"),
        Err(e) => error!("error while shutting down: {e}"),
    }
    Ok(())
}

#[tokio::test]
async fn server_functions_and_shuts_down_gracefully() {
    use hyper::{body::HttpBody, Client};
    use std::sync::Arc;
    use tokio::sync::Notify;

    let port = 1337;
    let shutdown = Arc::new(Notify::new());
    let shutdown_clone = shutdown.clone();
    let server = tokio::spawn(async move {
        prometheus_server(port, shutdown_clone.notified()).await.unwrap();
    });

    let count = 7;
    for _ in 0..count {
        TOTAL_UNSUCCESSFUL_EVENT_POSTS.inc();
    }

    let client = Client::new();
    let mut res = client
        .get(format!("http://localhost:{port}/").parse().unwrap())
        .await
        .unwrap();
    let mut buffer = String::new();
    while let Some(chunk) = res.body_mut().data().await {
        buffer += &String::from_utf8_lossy(&chunk.unwrap().to_vec());
    }

    let expected_count = count + 2; // TODO: Figure out why the +2...
    let expected_output = format!("hahaha_total_unsuccessful_event_posts {}", expected_count);
    assert!(buffer.contains(expected_output.as_str()));

    shutdown.notify_one();
    let ret = server.await;
    assert!(ret.is_ok())
}
