use std::fmt::Debug;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use async_trait::async_trait;
use futures_util::future::BoxFuture;
use tower::{Service, ServiceExt};
use tracing::error;
use uuid::Uuid;

use crate::CommonError;

pub fn generate_uuid() -> String {
    let uuid_str = Uuid::new_v4().to_string();
    uuid_str.replace('-', "")
}

pub async fn ready_and_call_service<T, S>(
    service: &mut S,
    request: T,
) -> Result<S::Response, S::Error>
where
    S: Service<T>,
    S::Error: Debug,
{
    let service_ready = match service.ready().await {
        Ok(v) => v,
        Err(e) => {
            return Err(e);
        }
    };
    match service_ready.call(request).await {
        Ok(v) => Ok(v),
        Err(e) => {
            return Err(e);
        }
    }
}
