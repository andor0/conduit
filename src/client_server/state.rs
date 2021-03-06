use super::State;
use crate::{pdu::PduBuilder, ConduitResult, Database, Error, Ruma};
use ruma::{
    api::client::{
        error::ErrorKind,
        r0::state::{
            get_state_events, get_state_events_for_empty_key, get_state_events_for_key,
            send_state_event_for_empty_key, send_state_event_for_key,
        },
    },
    events::{room::canonical_alias, EventType},
    Raw,
};

#[cfg(feature = "conduit_bin")]
use rocket::{get, put};

#[cfg_attr(
    feature = "conduit_bin",
    put("/_matrix/client/r0/rooms/<_>/state/<_>/<_>", data = "<body>")
)]
pub fn send_state_event_for_key_route(
    db: State<'_, Database<'_>>,
    body: Ruma<send_state_event_for_key::IncomingRequest>,
) -> ConduitResult<send_state_event_for_key::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    let content = serde_json::from_str::<serde_json::Value>(
        body.json_body
            .as_ref()
            .ok_or(Error::BadRequest(ErrorKind::BadJson, "Invalid JSON body."))?
            .get(),
    )
    .map_err(|_| Error::BadRequest(ErrorKind::BadJson, "Invalid JSON body."))?;

    if body.event_type == EventType::RoomCanonicalAlias {
        let canonical_alias = serde_json::from_value::<
            Raw<canonical_alias::CanonicalAliasEventContent>,
        >(content.clone())
        .expect("from_value::<Raw<..>> can never fail")
        .deserialize()
        .map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Invalid canonical alias."))?;

        let mut aliases = canonical_alias.alt_aliases;

        if let Some(alias) = canonical_alias.alias {
            aliases.push(alias);
        }

        for alias in aliases {
            if alias.server_name() != db.globals.server_name()
                || db
                    .rooms
                    .id_from_alias(&alias)?
                    .filter(|room| room == &body.room_id) // Make sure it's the right room
                    .is_none()
            {
                return Err(Error::BadRequest(
                    ErrorKind::Forbidden,
                    "You are only allowed to send canonical_alias \
                    events when it's aliases already exists",
                ));
            }
        }
    }

    let event_id = db.rooms.append_pdu(
        PduBuilder {
            room_id: body.room_id.clone(),
            sender: sender_id.clone(),
            event_type: body.event_type.clone(),
            content,
            unsigned: None,
            state_key: Some(body.state_key.clone()),
            redacts: None,
        },
        &db.globals,
        &db.account_data,
    )?;

    Ok(send_state_event_for_key::Response { event_id }.into())
}

#[cfg_attr(
    feature = "conduit_bin",
    put("/_matrix/client/r0/rooms/<_>/state/<_>", data = "<body>")
)]
pub fn send_state_event_for_empty_key_route(
    db: State<'_, Database<'_>>,
    body: Ruma<send_state_event_for_empty_key::IncomingRequest>,
) -> ConduitResult<send_state_event_for_empty_key::Response> {
    // This just calls send_state_event_for_key_route
    let Ruma {
        body:
            send_state_event_for_empty_key::IncomingRequest {
                room_id,
                event_type,
                data,
            },
        sender_id,
        device_id,
        json_body,
    } = body;

    Ok(send_state_event_for_empty_key::Response {
        event_id: send_state_event_for_key_route(
            db,
            Ruma {
                body: send_state_event_for_key::IncomingRequest {
                    room_id,
                    event_type,
                    data,
                    state_key: "".to_owned(),
                },
                sender_id,
                device_id,
                json_body,
            },
        )?
        .0
        .event_id,
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/rooms/<_>/state", data = "<body>")
)]
pub fn get_state_events_route(
    db: State<'_, Database<'_>>,
    body: Ruma<get_state_events::Request>,
) -> ConduitResult<get_state_events::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    if !db.rooms.is_joined(sender_id, &body.room_id)? {
        return Err(Error::BadRequest(
            ErrorKind::Forbidden,
            "You don't have permission to view the room state.",
        ));
    }

    Ok(get_state_events::Response {
        room_state: db
            .rooms
            .room_state_full(&body.room_id)?
            .values()
            .map(|pdu| pdu.to_state_event())
            .collect(),
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/rooms/<_>/state/<_>/<_>", data = "<body>")
)]
pub fn get_state_events_for_key_route(
    db: State<'_, Database<'_>>,
    body: Ruma<get_state_events_for_key::Request>,
) -> ConduitResult<get_state_events_for_key::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    if !db.rooms.is_joined(sender_id, &body.room_id)? {
        return Err(Error::BadRequest(
            ErrorKind::Forbidden,
            "You don't have permission to view the room state.",
        ));
    }

    let event = db
        .rooms
        .room_state_get(&body.room_id, &body.event_type, &body.state_key)?
        .ok_or(Error::BadRequest(
            ErrorKind::NotFound,
            "State event not found.",
        ))?;

    Ok(get_state_events_for_key::Response {
        content: serde_json::value::to_raw_value(&event.content)
            .map_err(|_| Error::bad_database("Invalid event content in database"))?,
    }
    .into())
}

#[cfg_attr(
    feature = "conduit_bin",
    get("/_matrix/client/r0/rooms/<_>/state/<_>", data = "<body>")
)]
pub fn get_state_events_for_empty_key_route(
    db: State<'_, Database<'_>>,
    body: Ruma<get_state_events_for_empty_key::Request>,
) -> ConduitResult<get_state_events_for_empty_key::Response> {
    let sender_id = body.sender_id.as_ref().expect("user is authenticated");

    if !db.rooms.is_joined(sender_id, &body.room_id)? {
        return Err(Error::BadRequest(
            ErrorKind::Forbidden,
            "You don't have permission to view the room state.",
        ));
    }

    let event = db
        .rooms
        .room_state_get(&body.room_id, &body.event_type, "")?
        .ok_or(Error::BadRequest(
            ErrorKind::NotFound,
            "State event not found.",
        ))?;

    Ok(get_state_events_for_empty_key::Response {
        content: serde_json::value::to_raw_value(&event)
            .map_err(|_| Error::bad_database("Invalid event content in database"))?,
    }
    .into())
}
