use crate::state::AppState;
use actix::Addr;
use actix_web::web::{Data, Path, Payload};
use actix_web::{get, web, HttpRequest, HttpResponse, Result, Scope};
use actix_web_actors::ws;
use std::sync::Arc;

use realtime::client::{ClientSession, RealtimeUserImpl};
use realtime::collaborate::CollabServer;

use crate::biz::collab::access_control::CollabAccessControlImpl;
use crate::biz::collab::storage::CollabPostgresDBStorage;
use crate::component::auth::jwt::{authorization_from_token, UserUuid};
use database::user::select_uid_from_uuid;
use shared_entity::response::AppResponseError;
use std::time::Duration;
use tracing::instrument;

pub fn ws_scope() -> Scope {
  web::scope("/ws").service(establish_ws_connection)
}

const MAX_FRAME_SIZE: usize = 65_536; // 64 KiB

type CollabServerData = Data<
  Addr<CollabServer<CollabPostgresDBStorage, Arc<RealtimeUserImpl>, Arc<CollabAccessControlImpl>>>,
>;

#[instrument(skip_all, err)]
#[get("/{token}/{device_id}")]
pub async fn establish_ws_connection(
  request: HttpRequest,
  payload: Payload,
  path: Path<(String, String)>,
  state: Data<AppState>,
  server: CollabServerData,
) -> Result<HttpResponse> {
  tracing::info!("receive ws connect: {:?}", request);
  let (token, device_id) = path.into_inner();
  let auth = authorization_from_token(token.as_str(), &state)?;
  let user_uuid = UserUuid::from_auth(auth)?;
  let uid = select_uid_from_uuid(&state.pg_pool, &user_uuid)
    .await
    .map_err(AppResponseError::from)?;
  let realtime_user = Arc::new(RealtimeUserImpl::new(uid, user_uuid.to_string(), device_id));
  let client = ClientSession::new(
    realtime_user,
    server.get_ref().clone(),
    Duration::from_secs(state.config.websocket.heartbeat_interval as u64),
    Duration::from_secs(state.config.websocket.client_timeout as u64),
  );

  match ws::WsResponseBuilder::new(client, &request, payload)
    .frame_size(MAX_FRAME_SIZE * 2)
    .start()
  {
    Ok(response) => Ok(response),
    Err(e) => {
      tracing::error!("🔴ws connection error: {:?}", e);
      Err(e)
    },
  }
}
