#[macro_use]
extern crate lazy_static;

use futures::{StreamExt, TryStreamExt};
use k8s_openapi::api::core::v1::Pod;
use kube::{
    api::{Api, ListParams},
    runtime::{events::Reporter, utils::try_flatten_applied, watcher},
    Client, Resource, ResourceExt,
};
use std::sync::Arc;
use tokio::sync::Notify;
use tracing::{error, info, warn};

mod actions;
mod api;
mod events;
mod pod;
mod prometheus;

use crate::{api::Destroyer, events::Recorder, pod::Sidecars, prometheus::*};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().json().try_init().unwrap();

    let actions = actions::generate();
    let client = Client::try_default().await?;

    let pods: Api<Pod> = Api::all(client.clone());
    let lp = ListParams::default().labels("nais.io/naisjob=true");

    let h = hostname::get()?;
    let host_name = match h.to_str() {
        Some(s) => s,
        None => "hahaha-1337", // consider dying here, this should never happen after all.
    };

    let reporter = Reporter {
        controller: "hahaha".into(),
        instance: Some(host_name.into()),
    };

    let mut ew = try_flatten_applied(watcher(pods, lp)).boxed();

    let shutdown = Arc::new(Notify::new());
    let shutdown_clone = shutdown.clone();
    let prom = tokio::spawn(async move {
        prometheus_server(8999, shutdown_clone.notified()).await.unwrap();
    });

    while let Some(pod) = ew.try_next().await? {
        let pod_name = pod.name();

        let running_sidecars = pod.sidecars().unwrap_or_else(|err| {
            info!("Getting running sidecars for {pod_name}: {err}");
            Vec::new()
        });
        if running_sidecars.is_empty() {
            // Move onto the next iteration if there's nothing to look at
            continue;
        }

        let namespace = match pod.namespace() {
            Some(namespace) => namespace,
            None => "default".into(),
        };
        // we need a namespaced api to `exec` and `portforward` into the target pod.
        let api: Api<Pod> = Api::namespaced(client.clone(), &namespace);

        // set up a recorder for publishing events to the Pod
        let recorder = Recorder::new(client.clone(), reporter.clone(), pod.object_ref(&()));

        info!("{pod_name} in namespace {namespace} needs help shutting down some residual containers!");

        let job_name = match pod.job_name() {
            Ok(name) => name,
            Err(e) => {
                warn!("Getting job name from pod: {e}");
                continue;
            }
        };

        for sidecar in running_sidecars {
            let sidecar_name = sidecar.name;
            info!("Found {}", &sidecar_name);
            let action = match actions.get(&sidecar_name) {
                Some(action) => action,
                None => {
                    warn!("I don't know how to shut down {sidecar_name} (in {pod_name} in namespace {namespace})");
                    continue;
                }
            };
            let res = api.shutdown(action, &pod_name, &sidecar_name).await;
            if let Err(err) = res {
                error!("Couldn't shutdown: {err}");
                if let Err(e) = recorder
                    .warn(format!("Unsuccessfully shut down container {sidecar_name}: {err}"))
                    .await
                {
                    error!("Couldn't publish Kubernetes Event: {e}");
                    TOTAL_UNSUCCESSFUL_EVENT_POSTS.inc();
                }
                FAILED_SIDECAR_SHUTDOWNS
                    .with_label_values(&[&sidecar_name, &job_name, &namespace])
                    .inc();
                continue;
            }
            if let Err(e) = recorder.info(format!("Shut down container {sidecar_name}")).await {
                error!("Couldn't publish Kubernetes Event: {e}");
                continue;
            }
            SIDECAR_SHUTDOWNS
                .with_label_values(&[&sidecar_name, &job_name, &namespace])
                .inc();
        }
    }

    // we're likely not ever reaching down here, but let's be nice about it if we do
    shutdown.notify_one();
    prom.await?;
    Ok(())
}
