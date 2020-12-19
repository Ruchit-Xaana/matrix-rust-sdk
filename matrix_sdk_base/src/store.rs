use std::{collections::BTreeMap, convert::TryFrom, path::Path, time::SystemTime};

use futures::stream::{self, Stream};
use matrix_sdk_common::{
    events::{
        presence::PresenceEvent, room::member::MembershipState, AnyBasicEvent,
        AnyStrippedStateEvent, AnySyncStateEvent, EventContent, EventType,
    },
    identifiers::{RoomId, UserId},
};

use sled::{transaction::TransactionResult, Config, Db, Transactional, Tree};
use tracing::info;

use crate::{
    responses::{MemberEvent, StrippedMemberEvent},
    rooms::RoomInfo,
    Session,
};

#[derive(Debug, Clone)]
pub struct Store {
    inner: Db,
    session: Tree,
    account_data: Tree,
    members: Tree,
    joined_user_ids: Tree,
    invited_user_ids: Tree,
    room_info: Tree,
    room_state: Tree,
    room_account_data: Tree,
    stripped_room_info: Tree,
    stripped_room_state: Tree,
    stripped_members: Tree,
    presence: Tree,
}

#[derive(Debug, Default)]
pub struct StateChanges {
    pub session: Option<Session>,
    pub account_data: BTreeMap<String, AnyBasicEvent>,
    pub presence: BTreeMap<UserId, PresenceEvent>,

    pub members: BTreeMap<RoomId, BTreeMap<UserId, MemberEvent>>,
    pub state: BTreeMap<RoomId, BTreeMap<String, BTreeMap<String, AnySyncStateEvent>>>,
    pub room_account_data: BTreeMap<RoomId, BTreeMap<String, AnyBasicEvent>>,
    pub room_infos: BTreeMap<RoomId, RoomInfo>,

    pub stripped_state: BTreeMap<RoomId, BTreeMap<String, BTreeMap<String, AnyStrippedStateEvent>>>,
    pub stripped_members: BTreeMap<RoomId, BTreeMap<UserId, StrippedMemberEvent>>,
    pub invited_room_info: BTreeMap<RoomId, RoomInfo>,
}

impl StateChanges {
    pub fn add_presence_event(&mut self, event: PresenceEvent) {
        self.presence.insert(event.sender.clone(), event);
    }

    pub fn add_room(&mut self, room: RoomInfo) {
        self.room_infos
            .insert(room.room_id.as_ref().to_owned(), room);
    }

    pub fn add_account_data(&mut self, event: AnyBasicEvent) {
        self.account_data
            .insert(event.content().event_type().to_owned(), event);
    }

    pub fn add_room_account_data(&mut self, room_id: &RoomId, event: AnyBasicEvent) {
        self.room_account_data
            .entry(room_id.to_owned())
            .or_insert_with(BTreeMap::new)
            .insert(event.content().event_type().to_owned(), event);
    }

    pub fn add_stripped_state_event(&mut self, room_id: &RoomId, event: AnyStrippedStateEvent) {
        self.stripped_state
            .entry(room_id.to_owned())
            .or_insert_with(BTreeMap::new)
            .entry(event.content().event_type().to_string())
            .or_insert_with(BTreeMap::new)
            .insert(event.state_key().to_string(), event);
    }

    pub fn add_stripped_member(&mut self, room_id: &RoomId, event: StrippedMemberEvent) {
        let user_id = UserId::try_from(event.state_key.as_str()).unwrap();
        self.stripped_members
            .entry(room_id.to_owned())
            .or_insert_with(BTreeMap::new)
            .insert(user_id, event);
    }

    pub fn add_state_event(&mut self, room_id: &RoomId, event: AnySyncStateEvent) {
        self.state
            .entry(room_id.to_owned())
            .or_insert_with(BTreeMap::new)
            .entry(event.content().event_type().to_string())
            .or_insert_with(BTreeMap::new)
            .insert(event.state_key().to_string(), event);
    }
}

impl From<Session> for StateChanges {
    fn from(session: Session) -> Self {
        Self {
            session: Some(session),
            ..Default::default()
        }
    }
}

impl Store {
    fn open_helper(db: Db) -> Self {
        let session = db.open_tree("session").unwrap();
        let account_data = db.open_tree("account_data").unwrap();

        let members = db.open_tree("members").unwrap();
        let joined_user_ids = db.open_tree("joined_user_ids").unwrap();
        let invited_user_ids = db.open_tree("invited_user_ids").unwrap();

        let room_state = db.open_tree("room_state").unwrap();
        let room_info = db.open_tree("room_infos").unwrap();
        let presence = db.open_tree("presence").unwrap();
        let room_account_data = db.open_tree("room_account_data").unwrap();

        let stripped_room_info = db.open_tree("stripped_room_info").unwrap();
        let stripped_members = db.open_tree("stripped_members").unwrap();
        let stripped_room_state = db.open_tree("stripped_room_state").unwrap();

        Self {
            inner: db,
            session,
            account_data,
            members,
            joined_user_ids,
            invited_user_ids,
            room_account_data,
            presence,
            room_state,
            room_info,
            stripped_room_info,
            stripped_members,
            stripped_room_state,
        }
    }

    pub fn open() -> Self {
        let db = Config::new().temporary(true).open().unwrap();

        Store::open_helper(db)
    }

    pub fn open_with_path(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().join("matrix-sdk-state");
        let db = Config::new().temporary(false).path(path).open().unwrap();

        Store::open_helper(db)
    }

    pub async fn save_filter(&self, filter_name: &str, filter_id: &str) {
        self.session
            .insert(&format!("filter{}", filter_name), filter_id)
            .unwrap();
    }

    pub async fn get_filter(&self, filter_name: &str) -> Option<String> {
        self.session
            .get(&format!("filter{}", filter_name))
            .unwrap()
            .map(|f| String::from_utf8_lossy(&f).to_string())
    }

    pub async fn save_changes(&self, changes: &StateChanges) {
        let now = SystemTime::now();

        let ret: TransactionResult<()> = (
            &self.session,
            &self.account_data,
            &self.members,
            &self.joined_user_ids,
            &self.invited_user_ids,
            &self.room_info,
            &self.room_state,
            &self.room_account_data,
            &self.presence,
            &self.stripped_room_info,
            &self.stripped_members,
            &self.stripped_room_state,
        )
            .transaction(
                |(
                    session,
                    account_data,
                    members,
                    joined,
                    invited,
                    summaries,
                    state,
                    room_account_data,
                    presence,
                    striped_rooms,
                    stripped_members,
                    stripped_state,
                )| {
                    if let Some(s) = &changes.session {
                        session.insert("session", serde_json::to_vec(s).unwrap())?;
                    }

                    for (room, events) in &changes.members {
                        for event in events.values() {
                            let key = format!("{}{}", room.as_str(), event.state_key.as_str());

                            match event.content.membership {
                                MembershipState::Join => {
                                    joined.insert(key.as_str(), event.state_key.as_str())?;
                                    invited.remove(key.as_str())?;
                                }
                                MembershipState::Invite => {
                                    invited.insert(key.as_str(), event.state_key.as_str())?;
                                    joined.remove(key.as_str())?;
                                }
                                _ => {
                                    joined.remove(key.as_str())?;
                                    invited.remove(key.as_str())?;
                                }
                            }

                            members.insert(
                                format!("{}{}", room.as_str(), &event.state_key).as_str(),
                                serde_json::to_vec(&event).unwrap(),
                            )?;
                        }
                    }

                    for (event_type, event) in &changes.account_data {
                        account_data
                            .insert(event_type.as_str(), serde_json::to_vec(&event).unwrap())?;
                    }

                    for (room, events) in &changes.room_account_data {
                        for (event_type, event) in events {
                            room_account_data.insert(
                                format!("{}{}", room.as_str(), event_type).as_str(),
                                serde_json::to_vec(&event).unwrap(),
                            )?;
                        }
                    }

                    for (room, event_types) in &changes.state {
                        for events in event_types.values() {
                            for event in events.values() {
                                state.insert(
                                    format!(
                                        "{}{}{}",
                                        room.as_str(),
                                        event.content().event_type(),
                                        event.state_key(),
                                    )
                                    .as_bytes(),
                                    serde_json::to_vec(&event).unwrap(),
                                )?;
                            }
                        }
                    }

                    for (room_id, summary) in &changes.room_infos {
                        summaries
                            .insert(room_id.as_bytes(), serde_json::to_vec(summary).unwrap())?;
                    }

                    for (sender, event) in &changes.presence {
                        presence.insert(sender.as_bytes(), serde_json::to_vec(&event).unwrap())?;
                    }

                    for (room_id, info) in &changes.invited_room_info {
                        striped_rooms
                            .insert(room_id.as_str(), serde_json::to_vec(&info).unwrap())?;
                    }

                    for (room, events) in &changes.stripped_members {
                        for event in events.values() {
                            stripped_members.insert(
                                format!("{}{}", room.as_str(), &event.state_key).as_str(),
                                serde_json::to_vec(&event).unwrap(),
                            )?;
                        }
                    }

                    for (room, event_types) in &changes.stripped_state {
                        for events in event_types.values() {
                            for event in events.values() {
                                stripped_state.insert(
                                    format!(
                                        "{}{}{}",
                                        room.as_str(),
                                        event.content().event_type(),
                                        event.state_key(),
                                    )
                                    .as_bytes(),
                                    serde_json::to_vec(&event).unwrap(),
                                )?;
                            }
                        }
                    }

                    Ok(())
                },
            );

        ret.unwrap();

        self.inner.flush_async().await.unwrap();

        info!("Saved changes in {:?}", now.elapsed().unwrap());
    }

    pub async fn get_presence_event(&self, user_id: &UserId) -> Option<PresenceEvent> {
        self.presence
            .get(user_id.as_bytes())
            .unwrap()
            .map(|e| serde_json::from_slice(&e).unwrap())
    }

    pub async fn get_state_event(
        &self,
        room_id: &RoomId,
        event_type: EventType,
        state_key: &str,
    ) -> Option<AnySyncStateEvent> {
        self.room_state
            .get(format!("{}{}{}", room_id.as_str(), event_type, state_key).as_bytes())
            .unwrap()
            .map(|e| serde_json::from_slice(&e).unwrap())
    }

    pub async fn get_member_event(
        &self,
        room_id: &RoomId,
        state_key: &UserId,
    ) -> Option<MemberEvent> {
        self.members
            .get(format!("{}{}", room_id.as_str(), state_key.as_str()))
            .unwrap()
            .map(|v| serde_json::from_slice(&v).unwrap())
    }

    pub async fn get_invited_user_ids(&self, room_id: &RoomId) -> impl Stream<Item = UserId> {
        stream::iter(
            self.invited_user_ids
                .scan_prefix(room_id.as_bytes())
                .map(|u| {
                    UserId::try_from(String::from_utf8_lossy(&u.unwrap().1).to_string()).unwrap()
                }),
        )
    }

    pub async fn get_joined_user_ids(&self, room_id: &RoomId) -> impl Stream<Item = UserId> {
        stream::iter(
            self.joined_user_ids
                .scan_prefix(room_id.as_bytes())
                .map(|u| {
                    UserId::try_from(String::from_utf8_lossy(&u.unwrap().1).to_string()).unwrap()
                }),
        )
    }

    pub async fn get_room_infos(&self) -> impl Stream<Item = RoomInfo> {
        stream::iter(
            self.room_info
                .iter()
                .map(|r| serde_json::from_slice(&r.unwrap().1).unwrap()),
        )
    }

    pub fn get_session(&self) -> Option<Session> {
        self.session
            .get("session")
            .unwrap()
            .map(|s| serde_json::from_slice(&s).unwrap())
    }
}

#[cfg(test)]
mod test {
    use std::{convert::TryFrom, time::SystemTime};

    use matrix_sdk_common::{
        events::{
            room::member::{MemberEventContent, MembershipState},
            Unsigned,
        },
        identifiers::{room_id, user_id, DeviceIdBox, EventId, UserId},
    };
    use matrix_sdk_test::async_test;

    use super::{StateChanges, Store};
    use crate::{responses::MemberEvent, Session};

    fn user_id() -> UserId {
        user_id!("@example:localhost")
    }

    fn device_id() -> DeviceIdBox {
        "DEVICEID".into()
    }

    fn membership_event() -> MemberEvent {
        let content = MemberEventContent {
            avatar_url: None,
            displayname: None,
            is_direct: None,
            third_party_invite: None,
            membership: MembershipState::Join,
        };

        MemberEvent {
            event_id: EventId::try_from("$h29iv0s8:example.com").unwrap(),
            content,
            sender: user_id(),
            origin_server_ts: SystemTime::now(),
            state_key: user_id(),
            prev_content: None,
            unsigned: Unsigned::default(),
        }
    }

    #[async_test]
    async fn test_session_saving() {
        let session = Session {
            user_id: user_id(),
            device_id: device_id(),
            access_token: "TEST_TOKEN".to_owned(),
        };

        let store = Store::open();

        store.save_changes(&session.clone().into()).await;
        let stored_session = store.get_session().unwrap();

        assert_eq!(session, stored_session);
    }

    #[async_test]
    async fn test_member_saving() {
        let store = Store::open();
        let room_id = room_id!("!test:localhost");
        let user_id = user_id();

        assert!(store.get_member_event(&room_id, &user_id).await.is_none());
        let mut changes = StateChanges::default();
        changes
            .members
            .entry(room_id.clone())
            .or_default()
            .insert(user_id.clone(), membership_event());

        store.save_changes(&changes).await;
        assert!(store.get_member_event(&room_id, &user_id).await.is_some());
    }
}
