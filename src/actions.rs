use hyper::http::Method;
use hyper::Uri;
use std::collections::BTreeMap;
/// Generate the action `BTreeMap`
///
/// Modify this function to add or remove sidecar definitions and their associated shutdown procedures.
pub fn generate() -> BTreeMap<String, Action> {
    BTreeMap::from([
        (
            "cloudsql-proxy".into(),
            Action::Portforward(Method::POST, "/quitquitquit".parse::<Uri>().unwrap(), 9091),
        ),
        (
            "vks-sidecar".into(),
            Action::Exec("/bin/kill -s INT 1".split(' ').map(String::from).collect()),
        ),
        (
            "istio-proxy".into(),
            Action::Portforward(Method::POST, "/quitquitquit".parse::<Uri>().unwrap(), 15000),
        ),
        (
            "linkerd-proxy".into(),
            Action::Portforward(Method::POST, "/shutdown".parse::<Uri>().unwrap(), 4191),
        ),
    ])
}

#[derive(Debug)]
pub enum Action {
    Portforward(Method, Uri, u16),
    Exec(Vec<String>),
}
