use super::State;
use super::{DEVICE_ID_LENGTH, TOKEN_LENGTH};
use crate::{utils, ConduitResult, Database, Error, Ruma};
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use ruma::{
    api::client::{
        error::ErrorKind,
        r0::session::{get_login_types, login, logout, logout_all},
    },
    events::EventType,
    UserId,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Claims {
    sub: String,
    exp: usize,
}

#[cfg(feature = "conduit_bin")]
use rocket::{get, post};

/// # `GET /_matrix/client/r0/login`
///
/// Get the homeserver's supported login types. One of these should be used as the `type` field
/// when logging in.
#[cfg_attr(feature = "conduit_bin", get("/_matrix/client/r0/login"))]
pub fn get_login_types_route() -> ConduitResult<get_login_types::Response> {
    Ok(get_login_types::Response {
        flows: vec![get_login_types::LoginType::Password],
    }
    .into())
}

/// # `POST /_matrix/client/r0/login`
///
/// Authenticates the user and returns an access token it can use in subsequent requests.
///
/// - The returned access token is associated with the user and device
/// - Old access tokens of that device should be invalidated
/// - If `device_id` is unknown, a new device will be created
///
/// Note: You can use [`GET /_matrix/client/r0/login`](fn.get_supported_versions_route.html) to see
/// supported login types.
#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/login", data = "<body>")
)]
pub fn login_route(
    db: State<'_, Database<'_>>,
    body: Ruma<login::Request>,
) -> ConduitResult<login::Response> {
    // Validate login method
    let user_id = match &body.login_info {
        login::LoginInfo::Password { password } => {
            let username = if let login::UserInfo::MatrixId(matrix_id) = body.user.clone() {
                matrix_id
            } else {
                return Err(Error::BadRequest(ErrorKind::Forbidden, "Bad login type."));
            };
            let user_id = UserId::parse_with_server_name(username, db.globals.server_name())
                .map_err(|_| {
                    Error::BadRequest(ErrorKind::InvalidUsername, "Username is invalid.")
                })?;
            let hash = db.users.password_hash(&user_id)?.ok_or(Error::BadRequest(
                ErrorKind::Forbidden,
                "Wrong username or password.",
            ))?;

            if hash.is_empty() {
                return Err(Error::BadRequest(
                    ErrorKind::UserDeactivated,
                    "The user has been deactivated",
                ));
            }

            let hash_matches = argon2::verify_encoded(&hash, password.as_bytes()).unwrap_or(false);

            if !hash_matches {
                return Err(Error::BadRequest(
                    ErrorKind::Forbidden,
                    "Wrong username or password.",
                ));
            }

            user_id
        }
        login::LoginInfo::Token { token } => {
            let token = jsonwebtoken::decode::<Claims>(
                &token,
                &db.globals.jwt_decoding_key(),
                &jsonwebtoken::Validation::default(),
            )
            .map_err(|_| Error::BadRequest(ErrorKind::InvalidUsername, "Token is invalid."))?;
            let username = token.claims.sub;
            let user_id = UserId::parse_with_server_name(username, db.globals.server_name()).map_err(|_| {
                Error::BadRequest(ErrorKind::InvalidUsername, "Username is invalid.")
            })?;

            if !db.users.exists(&user_id)? {
                db.account_data.update(
                    None,
                    &user_id,
                    EventType::PushRules,
                    &ruma::events::push_rules::PushRulesEvent {
                        content: ruma::events::push_rules::PushRulesEventContent {
                            global: crate::push_rules::default_pushrules(&user_id),
                        },
                    },
                    &db.globals,
                )?;
                db.users.create(&user_id, &generate_random_password())?;
            }

            user_id
        }
    };

    // Generate new device id if the user didn't specify one
    let device_id = body
        .body
        .device_id
        .clone()
        .unwrap_or_else(|| utils::random_string(DEVICE_ID_LENGTH).into());

    // Generate a new token for the device
    let token = utils::random_string(TOKEN_LENGTH);

    db.users.create_device(
        &user_id,
        &device_id,
        &token,
        body.initial_device_display_name.clone(),
    )?;

    Ok(login::Response {
        user_id,
        access_token: token,
        home_server: Some(db.globals.server_name().to_owned()),
        device_id,
        well_known: None,
    }
    .into())
}

/// # `POST /_matrix/client/r0/logout`
///
/// Log out the current device.
///
/// - Invalidates the access token
/// - Deletes the device and most of it's data (to-device events, last seen, etc.)
#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/logout", data = "<body>")
)]
pub fn logout_route(
    db: State<'_, Database<'_>>,
    body: Ruma<logout::Request>,
) -> ConduitResult<logout::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");
    let device_id = body.device_id.as_ref().expect("user is authenticated");

    db.users.remove_device(&sender_id, device_id)?;

    Ok(logout::Response.into())
}

/// # `POST /_matrix/client/r0/logout/all`
///
/// Log out all devices of this user.
///
/// - Invalidates all access tokens
/// - Deletes devices and most of their data (to-device events, last seen, etc.)
///
/// Note: This is equivalent to calling [`GET /_matrix/client/r0/logout`](fn.logout_route.html)
/// from each device of this user.
#[cfg_attr(
    feature = "conduit_bin",
    post("/_matrix/client/r0/logout/all", data = "<body>")
)]
pub fn logout_all_route(
    db: State<'_, Database<'_>>,
    body: Ruma<logout_all::Request>,
) -> ConduitResult<logout_all::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    for device_id in db.users.all_device_ids(sender_id) {
        if let Ok(device_id) = device_id {
            db.users.remove_device(&sender_id, &device_id)?;
        }
    }

    Ok(logout_all::Response.into())
}

fn generate_random_password() -> String {
    const LENGTH: usize = 40;

    thread_rng().sample_iter(&Alphanumeric).take(LENGTH).collect()
}
