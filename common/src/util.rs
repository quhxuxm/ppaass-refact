use std::{any::type_name, fmt::Debug};

use futures_util::TryFutureExt;
use tower::{Service, ServiceExt};
use tracing::error;
use uuid::Uuid;

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
    let service_type_name = type_name::<S>();
    match service.ready().and_then(|v| v.call(request)).await {
        Ok(v) => Ok(v),
        Err(e) => {
            error!(
                "Fail to invoke service [{}] because of errors: {:#?}",
                service_type_name, e
            );
            Err(e)
        }
    }
}
