use std::{any::type_name, fmt::Debug};

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
    let service_ready = match service.ready().await {
        Ok(v) => v,
        Err(e) => {
            error!(
                "Fail to invoke service: {:#?}\nErrors(not ready): {:#?}",
                service_type_name, e
            );
            return Err(e);
        }
    };
    match service_ready.call(request).await {
        Ok(v) => Ok(v),
        Err(e) => {
            error!(
                "Fail to invoke service: {:#?}\nErrors(on call): {:#?}",
                service_type_name, e
            );
            return Err(e);
        }
    }
}
