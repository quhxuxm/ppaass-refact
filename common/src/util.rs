use std::fmt::Debug;
use std::marker::PhantomData;

use tower::{Service, ServiceExt};
use tracing::error;
use uuid::Uuid;

use crate::CommonError;

pub fn generate_uuid() -> String {
    let uuid_str = Uuid::new_v4().to_string();
    uuid_str.replace('-', "")
}

pub struct CallServiceResult<S, T, U>
where S: Service<T, Response = U, Error = CommonError> + Debug + ServiceExt<T>, {
    pub service: S,
    pub result: U,
    _mark: PhantomData<T>,
}

pub async fn general_call_service<T, U, S>(
    mut service: S,
    request: T,
) -> Result<CallServiceResult<S, T, U>, S::Error>
where S: Service<T, Response = U, Error = CommonError> + Debug + ServiceExt<T>, {
    let service_ready = match service.ready().await {
        Ok(v) => v,
        Err(e) => {
            error!("Fail to service {:#?} because of error: {:#?}", service, e);
            return Err(e);
        }
    };
    match service_ready.call(request).await {
        Ok(v) => Ok(CallServiceResult {
            service,
            result: v,
            _mark: PhantomData,
        }),
        Err(e) => {
            error!("Fail to service {:#?} because of error: {:#?}", service, e);
            return Err(e);
        }
    }
}
