use std::fmt::Debug;

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
    S: Service<T> + Debug,
    S::Error: Debug,
{
    let service_ready = match service.ready().await {
        Ok(v) => v,
        Err(e) => {
            error!(
                "Fail to invoke service: {:#?} because of error(not ready): {:#?}",
                service, e
            );
            return Err(e);
        }
    };
    match service_ready.call(request).await {
        Ok(v) => Ok(v),
        Err(e) => {
            error!(
                "Fail to invoke service: {:#?} because of error(on call): {:#?}",
                service, e
            );
            return Err(e);
        }
    }
}
