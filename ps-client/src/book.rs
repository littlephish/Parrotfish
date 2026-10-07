use std::collections::{HashMap, HashSet};

use ps_protocol::command::Command;

pub const STANDARD_ICONS: [u32; 5] = [100, 200, 300, 500, 600];

pub fn is_standard_icon(id: u32) -> bool {
    STANDARD_ICONS.contains(&id)
}

pub fn icon_id(raw: &str) -> u32 {
    let text = raw.trim();
    if let Ok(value) = text.parse::<u64>() {
        return value as u32;
    }
    text.parse::<i64>().map(|value| value as u32).unwrap_or(0)
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Channel {
    pub id: u64,
    pub parent: u64,
    pub order: u64,
    pub name: String,
    pub topic: String,
    pub codec: u8,
    pub codec_quality: u8,
    pub unencrypted: bool,
    pub is_default: bool,
    pub has_password: bool,
    pub max_clients: i64,
    pub needed_talk_power: i64,
    pub icon: u32,
    pub description: String,
    pub description_known: bool,
}

impl Channel {
    pub fn apply(&mut self, cmd: &Command, item: usize) {
        if let Some(v) = cmd.num_at(item, "cpid") {
            self.parent = v;
        }
        if let Some(v) = cmd.num_at(item, "channel_order") {
            self.order = v;
        }
        if let Some(v) = cmd.num_at(item, "order") {
            self.order = v;
        }
        if let Some(v) = cmd.get_at(item, "channel_name") {
            self.name = v.to_string();
        }
        if let Some(v) = cmd.get_at(item, "channel_topic") {
            self.topic = v.to_string();
        }
        if let Some(v) = cmd.num_at(item, "channel_codec") {
            self.codec = v;
        }
        if let Some(v) = cmd.num_at(item, "channel_codec_quality") {
            self.codec_quality = v;
        }
        if let Some(v) = cmd.bool_at(item, "channel_codec_is_unencrypted") {
            self.unencrypted = v;
        }
        if let Some(v) = cmd.bool_at(item, "channel_flag_default") {
            self.is_default = v;
        }
        if let Some(v) = cmd.bool_at(item, "channel_flag_password") {
            self.has_password = v;
        }
        if let Some(v) = cmd.num_at(item, "channel_maxclients") {
            self.max_clients = v;
        }
        if let Some(v) = cmd.num_at(item, "channel_needed_talk_power") {
            self.needed_talk_power = v;
        }
        if let Some(v) = cmd.get_at(item, "channel_icon_id") {
            self.icon = icon_id(v);
        }
        if let Some(v) = cmd.get_own(item, "channel_description") {
            self.description = v.to_string();
            self.description_known = true;
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClientInfo {
    pub id: u16,
    pub channel: u64,
    pub nickname: String,
    pub uid: String,
    pub input_muted: bool,
    pub output_muted: bool,
    pub input_hardware: bool,
    pub output_hardware: bool,
    pub away: bool,
    pub away_message: String,
    pub is_query: bool,
    pub talk_power: i64,
    pub is_talker: bool,
    pub is_recording: bool,
    pub is_channel_commander: bool,
    pub talking: bool,
    pub whispering: bool,
    pub icon: u32,
    pub server_groups: Vec<u64>,
    pub channel_group: u64,
    pub icons: Vec<u32>,
    pub database_id: u64,
    pub country: String,
    pub version: String,
    pub platform: String,
    pub description: String,
    pub talk_request: bool,
    pub talk_request_message: String,
    pub is_priority_speaker: bool,
}

impl ClientInfo {
    pub fn apply(&mut self, cmd: &Command, item: usize) {
        if let Some(v) = cmd.get_at(item, "client_nickname") {
            self.nickname = v.to_string();
        }
        if let Some(v) = cmd.get_at(item, "client_unique_identifier") {
            self.uid = v.to_string();
        }
        if let Some(v) = cmd.bool_at(item, "client_input_muted") {
            self.input_muted = v;
        }
        if let Some(v) = cmd.bool_at(item, "client_output_muted") {
            self.output_muted = v;
        }
        if let Some(v) = cmd.bool_at(item, "client_input_hardware") {
            self.input_hardware = v;
        }
        if let Some(v) = cmd.bool_at(item, "client_output_hardware") {
            self.output_hardware = v;
        }
        if let Some(v) = cmd.bool_at(item, "client_away") {
            self.away = v;
        }
        if let Some(v) = cmd.get_at(item, "client_away_message") {
            self.away_message = v.to_string();
        }
        if let Some(v) = cmd.num_at::<u8>(item, "client_type") {
            self.is_query = v == 1;
        }
        if let Some(v) = cmd.num_at(item, "client_talk_power") {
            self.talk_power = v;
        }
        if let Some(v) = cmd.bool_at(item, "client_is_talker") {
            self.is_talker = v;
        }
        if let Some(v) = cmd.bool_at(item, "client_is_recording") {
            self.is_recording = v;
        }
        if let Some(v) = cmd.bool_at(item, "client_is_channel_commander") {
            self.is_channel_commander = v;
        }
        if let Some(v) = cmd.get_at(item, "client_icon_id") {
            self.icon = icon_id(v);
        }
        if let Some(v) = cmd.num_at(item, "client_database_id") {
            self.database_id = v;
        }
        if let Some(v) = cmd.get_at(item, "client_country") {
            self.country = v.to_string();
        }
        if let Some(v) = cmd.get_at(item, "client_version") {
            self.version = v.to_string();
        }
        if let Some(v) = cmd.get_at(item, "client_platform") {
            self.platform = v.to_string();
        }
        if let Some(v) = cmd.get_at(item, "client_description") {
            self.description = v.to_string();
        }
        if let Some(v) = cmd.get_at(item, "client_talk_request") {
            self.talk_request = v.trim().parse::<i64>().map(|at| at != 0).unwrap_or(false);
        }
        if let Some(v) = cmd.get_at(item, "client_talk_request_msg") {
            self.talk_request_message = v.to_string();
        }
        if let Some(v) = cmd.bool_at(item, "client_is_priority_speaker") {
            self.is_priority_speaker = v;
        }
        if let Some(v) = cmd.num_at(item, "client_channel_group_id") {
            self.channel_group = v;
        }
        if let Some(v) = cmd.get_at(item, "client_servergroups") {
            let mut groups: Vec<u64> = Vec::new();
            for group in v.split(',').filter_map(|part| part.trim().parse().ok()) {
                if !groups.contains(&group) {
                    groups.push(group);
                }
            }
            self.server_groups = groups;
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ServerInfo {
    pub name: String,
    pub welcome_message: String,
    pub platform: String,
    pub version: String,
    pub virtual_server_id: u64,
    pub max_clients: u32,
    pub codec_encryption_mode: u8,
    pub uid: String,
    pub icon: u32,
    pub host_message: String,
    pub host_message_mode: u8,
}

impl ServerInfo {
    pub fn apply(&mut self, cmd: &Command) {
        if let Some(v) = cmd.get("virtualserver_name") {
            self.name = v.to_string();
        }
        if let Some(v) = cmd.get("virtualserver_welcomemessage") {
            self.welcome_message = v.to_string();
        }
        if let Some(v) = cmd.get("virtualserver_platform") {
            self.platform = v.to_string();
        }
        if let Some(v) = cmd.get("virtualserver_version") {
            self.version = v.to_string();
        }
        if let Some(v) = cmd.num("virtualserver_id") {
            self.virtual_server_id = v;
        }
        if let Some(v) = cmd.num("virtualserver_maxclients") {
            self.max_clients = v;
        }
        if let Some(v) = cmd.num("virtualserver_codec_encryption_mode") {
            self.codec_encryption_mode = v;
        }
        if let Some(v) = cmd.get("virtualserver_icon_id") {
            self.icon = icon_id(v);
        }
        if let Some(v) = cmd.get("virtualserver_hostmessage") {
            self.host_message = v.to_string();
        }
        if let Some(v) = cmd.num("virtualserver_hostmessage_mode") {
            self.host_message_mode = v;
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChannelNode {
    pub channel: Channel,
    pub depth: u32,
    pub clients: Vec<ClientInfo>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ServerView {
    pub server: ServerInfo,
    pub own_id: u16,
    pub own_channel: u64,
    pub channels: Vec<ChannelNode>,
}

impl ServerView {
    pub fn own_channel_node(&self) -> Option<&ChannelNode> {
        self.channels.iter().find(|n| n.channel.id == self.own_channel)
    }

    pub fn client(&self, id: u16) -> Option<&ClientInfo> {
        self.channels.iter().flat_map(|n| n.clients.iter()).find(|c| c.id == id)
    }

    pub fn client_count(&self) -> usize {
        self.channels.iter().map(|n| n.clients.len()).sum()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Group {
    pub id: u64,
    pub name: String,
    pub kind: u8,
    pub sort: u32,
    pub icon: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Book {
    pub server: ServerInfo,
    pub own_id: u16,
    pub channels: HashMap<u64, Channel>,
    pub clients: HashMap<u16, ClientInfo>,
    pub server_groups: HashMap<u64, Group>,
    pub channel_groups: HashMap<u64, Group>,
}

impl Book {
    pub fn own_channel(&self) -> u64 {
        self.clients.get(&self.own_id).map(|c| c.channel).unwrap_or(0)
    }

    pub fn voice_encryption(&self) -> bool {
        match self.server.codec_encryption_mode {
            1 => false,
            2 => true,
            _ => self
                .channels
                .get(&self.own_channel())
                .map(|c| !c.unencrypted)
                .unwrap_or(true),
        }
    }

    pub fn set_groups(&mut self, cmd: &Command, server: bool) {
        let key = if server { "sgid" } else { "cgid" };
        let mut groups = HashMap::new();
        for item in 0..cmd.len() {
            let Some(id) = cmd.num_at::<u64>(item, key) else {
                continue;
            };
            groups.insert(
                id,
                Group {
                    id,
                    name: cmd.get_at(item, "name").unwrap_or("").to_string(),
                    kind: cmd.num_at(item, "type").unwrap_or(0),
                    sort: cmd.num_at(item, "sortid").unwrap_or(0),
                    icon: cmd.get_at(item, "iconid").map(icon_id).unwrap_or(0),
                },
            );
        }
        if server {
            self.server_groups = groups;
        } else {
            self.channel_groups = groups;
        }
    }

    pub fn regular_groups(&self, server: bool) -> Vec<Group> {
        let source = if server { &self.server_groups } else { &self.channel_groups };
        let mut list: Vec<Group> = source.values().filter(|group| group.kind == 1).cloned().collect();
        list.sort_by(|a, b| (a.sort, a.id).cmp(&(b.sort, b.id)));
        list
    }

    pub fn group_member(&mut self, cmd: &Command, added: bool) {
        for item in 0..cmd.len() {
            let (Some(group), Some(id)) = (cmd.num_at::<u64>(item, "sgid"), cmd.num_at::<u16>(item, "clid")) else {
                continue;
            };
            if let Some(client) = self.clients.get_mut(&id) {
                client.server_groups.retain(|known| *known != group);
                if added {
                    client.server_groups.push(group);
                }
            }
        }
    }

    pub fn channel_group_changed(&mut self, cmd: &Command) {
        for item in 0..cmd.len() {
            let (Some(group), Some(id)) = (cmd.num_at::<u64>(item, "cgid"), cmd.num_at::<u16>(item, "clid")) else {
                continue;
            };
            if let Some(client) = self.clients.get_mut(&id) {
                client.channel_group = group;
            }
        }
    }

    fn client_icons(&self, client: &ClientInfo) -> Vec<u32> {
        let mut icons: Vec<u32> = Vec::new();
        let mut add = |id: u32| {
            if id != 0 && !icons.contains(&id) {
                icons.push(id);
            }
        };
        if let Some(group) = self.channel_groups.get(&client.channel_group) {
            add(group.icon);
        }
        let mut groups: Vec<&Group> =
            client.server_groups.iter().filter_map(|id| self.server_groups.get(id)).collect();
        groups.sort_by_key(|group| (group.sort, group.id));
        for group in groups {
            add(group.icon);
        }
        add(client.icon);
        icons
    }

    pub fn upsert_channels(&mut self, cmd: &Command) {
        for item in 0..cmd.len() {
            let Some(id) = cmd.num_at::<u64>(item, "cid") else {
                continue;
            };
            let channel = self.channels.entry(id).or_insert_with(|| Channel { id, ..Default::default() });
            channel.apply(cmd, item);
        }
    }

    pub fn description_changed(&mut self, cmd: &Command) {
        for item in 0..cmd.len() {
            if let Some(channel) = cmd.num_at::<u64>(item, "cid").and_then(|id| self.channels.get_mut(&id)) {
                channel.description_known = false;
            }
        }
    }

    pub fn remove_channels(&mut self, cmd: &Command) {
        for item in 0..cmd.len() {
            if let Some(id) = cmd.num_at::<u64>(item, "cid") {
                self.channels.remove(&id);
                self.clients.retain(|_, c| c.channel != id);
            }
        }
    }

    pub fn clients_entered(&mut self, cmd: &Command) -> Vec<ClientInfo> {
        let mut entered = Vec::new();
        for item in 0..cmd.len() {
            let Some(id) = cmd.num_at::<u16>(item, "clid") else {
                continue;
            };
            let client = self.clients.entry(id).or_insert_with(|| ClientInfo {
                id,
                input_hardware: true,
                output_hardware: true,
                ..Default::default()
            });
            if let Some(channel) = cmd.num_at(item, "ctid") {
                client.channel = channel;
            }
            client.apply(cmd, item);
            entered.push(client.clone());
        }
        entered
    }

    pub fn clients_moved(&mut self, cmd: &Command) -> Vec<(ClientInfo, u64, u64)> {
        let mut moved = Vec::new();
        for item in 0..cmd.len() {
            let (Some(id), Some(to)) =
                (cmd.num_at::<u16>(item, "clid"), cmd.num_at::<u64>(item, "ctid"))
            else {
                continue;
            };
            if let Some(client) = self.clients.get_mut(&id) {
                let from = client.channel;
                client.channel = to;
                client.talking = false;
                client.whispering = false;
                moved.push((client.clone(), from, to));
            }
        }
        moved
    }

    pub fn clients_left(&mut self, cmd: &Command) -> Vec<ClientInfo> {
        let mut left = Vec::new();
        for item in 0..cmd.len() {
            if let Some(id) = cmd.num_at::<u16>(item, "clid") {
                if let Some(client) = self.clients.remove(&id) {
                    left.push(client);
                }
            }
        }
        left
    }

    pub fn clients_updated(&mut self, cmd: &Command) {
        for item in 0..cmd.len() {
            if let Some(id) = cmd.num_at::<u16>(item, "clid") {
                if let Some(client) = self.clients.get_mut(&id) {
                    client.apply(cmd, item);
                }
            }
        }
    }

    pub fn set_talking(&mut self, id: u16, talking: bool, whisper: bool) -> bool {
        let whispering = talking && whisper;
        match self.clients.get_mut(&id) {
            Some(c) if c.talking != talking || c.whispering != whispering => {
                c.talking = talking;
                c.whispering = whispering;
                true
            }
            _ => false,
        }
    }

    pub fn channel_order(&self) -> Vec<(u64, u32)> {
        let mut children: HashMap<u64, Vec<&Channel>> = HashMap::new();
        for c in self.channels.values() {
            let parent = if c.parent != 0 && !self.channels.contains_key(&c.parent) { 0 } else { c.parent };
            children.entry(parent).or_default().push(c);
        }
        let mut out = Vec::with_capacity(self.channels.len());
        let mut visited = HashSet::new();
        self.walk(0, 0, &children, &mut visited, &mut out);
        let mut orphans: Vec<u64> =
            self.channels.keys().copied().filter(|id| !visited.contains(id)).collect();
        orphans.sort_unstable();
        for id in orphans {
            if visited.insert(id) {
                out.push((id, 0));
                self.walk(id, 1, &children, &mut visited, &mut out);
            }
        }
        out
    }

    fn walk(
        &self,
        parent: u64,
        depth: u32,
        children: &HashMap<u64, Vec<&Channel>>,
        visited: &mut HashSet<u64>,
        out: &mut Vec<(u64, u32)>,
    ) {
        let Some(kids) = children.get(&parent) else {
            return;
        };
        for id in sibling_order(kids) {
            if visited.insert(id) {
                out.push((id, depth));
                self.walk(id, depth + 1, children, visited, out);
            }
        }
    }

    pub fn view(&self) -> ServerView {
        let mut by_channel: HashMap<u64, Vec<ClientInfo>> = HashMap::new();
        for c in self.clients.values() {
            let mut client = c.clone();
            client.icons = self.client_icons(c);
            by_channel.entry(c.channel).or_default().push(client);
        }
        let channels = self
            .channel_order()
            .into_iter()
            .filter_map(|(id, depth)| {
                let channel = self.channels.get(&id)?.clone();
                let mut clients = by_channel.remove(&id).unwrap_or_default();
                clients.sort_by(|a, b| {
                    a.is_query
                        .cmp(&b.is_query)
                        .then_with(|| a.nickname.to_lowercase().cmp(&b.nickname.to_lowercase()))
                        .then_with(|| a.id.cmp(&b.id))
                });
                Some(ChannelNode { channel, depth, clients })
            })
            .collect();
        ServerView {
            server: self.server.clone(),
            own_id: self.own_id,
            own_channel: self.own_channel(),
            channels,
        }
    }
}

fn sibling_order(kids: &[&Channel]) -> Vec<u64> {
    let ids: HashSet<u64> = kids.iter().map(|c| c.id).collect();
    let mut after: HashMap<u64, Vec<u64>> = HashMap::new();
    for c in kids {
        let prev = if c.order != 0 && !ids.contains(&c.order) { 0 } else { c.order };
        after.entry(prev).or_default().push(c.id);
    }
    for v in after.values_mut() {
        v.sort_unstable();
    }
    let mut out = Vec::with_capacity(kids.len());
    let mut seen = HashSet::new();
    let mut stack: Vec<u64> = after.get(&0).cloned().unwrap_or_default();
    stack.reverse();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        out.push(id);
        if let Some(next) = after.get(&id) {
            for n in next.iter().rev() {
                stack.push(*n);
            }
        }
    }
    let mut rest: Vec<u64> = ids.into_iter().filter(|id| !seen.contains(id)).collect();
    rest.sort_unstable();
    out.extend(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book_from(list: &str) -> Book {
        let mut book = Book::default();
        book.upsert_channels(&Command::parse(list));
        book
    }

    #[test]
    fn orders_channels_by_previous_sibling() {
        let book = book_from(
            "channellist cid=1 cpid=0 channel_order=0 channel_name=Lobby|cid=5 cpid=0 channel_order=3 channel_name=Last|cid=3 cpid=0 channel_order=1 channel_name=Middle|cid=7 cpid=3 channel_order=0 channel_name=Sub\\sA|cid=8 cpid=3 channel_order=7 channel_name=Sub\\sB|cid=9 cpid=7 channel_order=0 channel_name=Deep",
        );
        assert_eq!(book.channel_order(), vec![(1, 0), (3, 0), (7, 1), (9, 2), (8, 1), (5, 0)]);
        let view = book.view();
        let names: Vec<&str> = view.channels.iter().map(|n| n.channel.name.as_str()).collect();
        assert_eq!(names, vec!["Lobby", "Middle", "Sub A", "Deep", "Sub B", "Last"]);
    }

    #[test]
    fn tolerates_broken_links() {
        let book = book_from(
            "channellist cid=1 cpid=0 channel_order=0|cid=2 cpid=0 channel_order=99|cid=3 cpid=0 channel_order=4|cid=4 cpid=0 channel_order=3|cid=6 cpid=42 channel_order=0",
        );
        let order = book.channel_order();
        let mut ids: Vec<u64> = order.iter().map(|(id, _)| *id).collect();
        assert_eq!(order.len(), 5);
        ids.sort_unstable();
        assert_eq!(ids, vec![1, 2, 3, 4, 6]);
        assert_eq!(order[0], (1, 0));
    }

    #[test]
    fn client_lifecycle() {
        let mut book = book_from(
            "channellist cid=1 cpid=0 channel_order=0 channel_name=Lobby channel_codec=4 channel_codec_quality=6 channel_codec_is_unencrypted=1|cid=2 cpid=0 channel_order=1 channel_name=Private channel_codec=5 channel_codec_quality=10 channel_codec_is_unencrypted=0 channel_flag_password=1",
        );
        book.own_id = 3;
        let entered = book.clients_entered(&Command::parse(
            "notifycliententerview cfid=0 ctid=1 reasonid=0 clid=3 client_unique_identifier=abc= client_nickname=Little\\sPhish client_input_muted=0 client_output_muted=0 client_type=0|clid=4 client_unique_identifier=def= client_nickname=bob client_type=0 client_input_muted=1|clid=5 ctid=2 client_nickname=Query client_type=1",
        ));
        assert_eq!(entered.len(), 3);
        assert_eq!(book.own_channel(), 1);
        assert_eq!(book.clients[&4].channel, 1);
        assert_eq!(book.clients[&4].nickname, "bob");
        assert!(book.clients[&4].input_muted);
        assert_eq!(book.clients[&5].channel, 2);
        assert!(book.clients[&5].is_query);
        assert!(!book.voice_encryption());

        let view = book.view();
        assert_eq!(view.own_channel, 1);
        assert_eq!(view.client_count(), 3);
        let lobby: Vec<&str> = view.channels[0].clients.iter().map(|c| c.nickname.as_str()).collect();
        assert_eq!(lobby, vec!["bob", "Little Phish"]);
        assert_eq!(view.own_channel_node().unwrap().channel.codec_quality, 6);

        let moved = book.clients_moved(&Command::parse("notifyclientmoved ctid=2 reasonid=0 clid=3"));
        assert_eq!(moved.len(), 1);
        assert_eq!((moved[0].1, moved[0].2), (1, 2));
        assert_eq!(book.own_channel(), 2);
        assert!(book.voice_encryption());

        book.server.codec_encryption_mode = 1;
        assert!(!book.voice_encryption());
        book.server.codec_encryption_mode = 2;
        assert!(book.voice_encryption());

        book.clients_updated(&Command::parse(
            "notifyclientupdated clid=4 client_nickname=Robert client_input_muted=0 client_away=1 client_away_message=brb",
        ));
        assert_eq!(book.clients[&4].nickname, "Robert");
        assert!(!book.clients[&4].input_muted);
        assert!(book.clients[&4].away);
        assert_eq!(book.clients[&4].away_message, "brb");

        assert!(book.set_talking(4, true, false));
        assert!(!book.set_talking(4, true, false));
        assert!(!book.set_talking(99, true, false));

        let left = book.clients_left(&Command::parse(
            "notifyclientleftview cfid=1 ctid=0 reasonid=8 reasonmsg=bye clid=4",
        ));
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].nickname, "Robert");
        assert!(!book.clients.contains_key(&4));

        book.remove_channels(&Command::parse("notifychanneldeleted invokerid=1 cid=2"));
        assert!(!book.channels.contains_key(&2));
        assert!(!book.clients.contains_key(&5));
    }

    #[test]
    fn whispers_are_marked_on_the_person() {
        let mut book = book_from("channellist cid=1 cpid=0 channel_order=0 channel_name=Lobby");
        book.clients_entered(&Command::parse("notifycliententerview cfid=0 ctid=1 reasonid=0 clid=4 client_nickname=Marlin client_type=0"));
        assert!(book.set_talking(4, true, true));
        assert!(book.clients[&4].talking && book.clients[&4].whispering);
        assert!(!book.set_talking(4, true, true));
        assert!(book.set_talking(4, true, false));
        assert!(!book.clients[&4].whispering);
        assert!(book.set_talking(4, false, false));
        assert!(!book.clients[&4].talking && !book.clients[&4].whispering);
    }

    #[test]
    fn group_lists_replace_each_other_and_sort() {
        let mut book = Book::default();
        book.set_groups(
            &Command::parse("notifyservergrouplist sgid=1 name=Guest\\sServer\\sQuery type=2 iconid=0 savedb=0 sortid=0|sgid=6 name=Server\\sAdmin type=1 iconid=300 savedb=1 sortid=20|sgid=8 name=Guest type=1 iconid=0 savedb=0 sortid=10|sgid=3 name=Template type=0 iconid=0 sortid=0"),
            true,
        );
        book.set_groups(
            &Command::parse("notifychannelgrouplist cgid=8 name=Guest type=1 iconid=0 sortid=0|cgid=5 name=Channel\\sAdmin type=1 iconid=100 sortid=0"),
            false,
        );
        let names: Vec<String> = book.regular_groups(true).into_iter().map(|g| g.name).collect();
        assert_eq!(names, vec!["Guest", "Server Admin"]);
        let channel: Vec<(u64, String)> = book.regular_groups(false).into_iter().map(|g| (g.id, g.name)).collect();
        assert_eq!(channel, vec![(5, "Channel Admin".to_string()), (8, "Guest".to_string())]);
        book.set_groups(&Command::parse("notifyservergrouplist sgid=9 name=Crew type=1 sortid=0"), true);
        assert_eq!(book.server_groups.len(), 1);
        assert_eq!(book.regular_groups(true)[0].id, 9);
        assert_eq!(book.channel_groups.len(), 2);
    }

    #[test]
    fn icon_ids_fold_to_32_bits() {
        assert_eq!(icon_id("0"), 0);
        assert_eq!(icon_id("100"), 100);
        assert_eq!(icon_id("2154984321"), 2154984321);
        assert_eq!(icon_id("-2139982975"), 2154984321);
        assert_eq!(icon_id("18446744071569568641"), 2154984321);
        assert_eq!(icon_id(" 452340182 "), 452340182);
        assert_eq!(icon_id(""), 0);
        assert_eq!(icon_id("icon"), 0);
        assert!(is_standard_icon(100) && is_standard_icon(600) && !is_standard_icon(400) && !is_standard_icon(0));
    }

    #[test]
    fn people_carry_group_and_own_icons() {
        let mut book = book_from(
            "channellist cid=1 cpid=0 channel_order=0 channel_name=Lobby channel_icon_id=0|cid=2 cpid=0 channel_order=1 channel_name=Deep\\sRock channel_icon_id=2154984321",
        );
        book.server.apply(&Command::parse("initserver virtualserver_name=Reef virtualserver_icon_id=-2139982975 aclid=3"));
        assert_eq!(book.server.icon, 2154984321);
        assert_eq!(book.channels[&2].icon, 2154984321);
        book.set_groups(
            &Command::parse("notifyservergrouplist sgid=6 name=Server\\sAdmin type=1 iconid=300 sortid=10|sgid=8 name=Guest type=1 iconid=452340182 sortid=20|sgid=9 name=Plain type=1 iconid=0 sortid=5"),
            true,
        );
        book.set_groups(
            &Command::parse("notifychannelgrouplist cgid=5 name=Channel\\sAdmin type=1 iconid=100 sortid=0|cgid=8 name=Guest type=1 iconid=0 sortid=0"),
            false,
        );
        book.own_id = 3;
        book.clients_entered(&Command::parse(
            "notifycliententerview cfid=0 ctid=1 reasonid=0 clid=3 client_nickname=Minnow client_type=0 client_servergroups=8,6,9 client_channel_group_id=5 client_icon_id=-2139982975|clid=4 client_nickname=Pike client_type=0 client_servergroups=8 client_channel_group_id=8 client_icon_id=0",
        ));
        assert_eq!(book.clients[&3].server_groups, vec![8, 6, 9]);
        let view = book.view();
        assert_eq!(view.server.icon, 2154984321);
        assert_eq!(view.channels[1].channel.icon, 2154984321);
        assert_eq!(view.client(3).unwrap().icons, vec![100, 300, 452340182, 2154984321]);
        assert_eq!(view.client(4).unwrap().icons, vec![452340182]);

        book.group_member(&Command::parse("notifyservergroupclientdeleted name=Server\\sAdmin sgid=6 invokerid=1 invokername=x clid=3 cluid=u"), false);
        book.channel_group_changed(&Command::parse("notifyclientchannelgroupchanged invokerid=1 invokername=x cgid=8 cgi=1 cid=1 clid=3"));
        assert_eq!(book.view().client(3).unwrap().icons, vec![452340182, 2154984321]);

        book.group_member(&Command::parse("notifyservergroupclientadded name=Server\\sAdmin sgid=6 invokerid=1 invokername=x clid=4 cluid=u"), true);
        book.group_member(&Command::parse("notifyservergroupclientadded name=Server\\sAdmin sgid=6 invokerid=1 invokername=x clid=4 cluid=u"), true);
        assert_eq!(book.clients[&4].server_groups, vec![8, 6]);
        book.clients_updated(&Command::parse("notifyclientupdated clid=3 client_icon_id=0"));
        book.upsert_channels(&Command::parse("notifychanneledited cid=1 reasonid=10 invokerid=1 channel_icon_id=452340182"));
        let view = book.view();
        assert_eq!(view.client(3).unwrap().icons, vec![452340182]);
        assert_eq!(view.client(4).unwrap().icons, vec![300, 452340182]);
        assert_eq!(view.channels[0].channel.icon, 452340182);

        book.clients_updated(&Command::parse("notifyclientupdated clid=4 client_servergroups=9,junk,,6 client_channel_group_id=5"));
        assert_eq!(book.clients[&4].server_groups, vec![9, 6]);
        assert_eq!(book.view().client(4).unwrap().icons, vec![100, 300]);

        book.set_groups(&Command::parse("notifyservergrouplist sgid=8 name=Guest type=1 iconid=0 sortid=20"), true);
        book.channel_group_changed(&Command::parse("notifyclientchannelgroupchanged cgid=8 cgi=1 cid=1 clid=4|cgid=5 clid=99"));
        assert!(book.view().client(4).unwrap().icons.is_empty());
    }

    #[test]
    fn details_about_a_person_are_kept() {
        let mut book = book_from("channellist cid=1 cpid=0 channel_order=0 channel_name=Lobby");
        book.clients_entered(&Command::parse(
            "notifycliententerview cfid=0 ctid=1 reasonid=0 clid=4 client_nickname=Marlin client_type=0 client_country=DE client_description=reef\\skeeper client_talk_request=0 client_is_priority_speaker=1 client_database_id=17",
        ));
        let marlin = &book.clients[&4];
        assert_eq!((marlin.country.as_str(), marlin.description.as_str()), ("DE", "reef keeper"));
        assert!(marlin.is_priority_speaker && !marlin.talk_request && marlin.version.is_empty());
        assert_eq!(marlin.database_id, 17);
        book.clients_updated(&Command::parse(
            "notifyclientupdated clid=4 client_version=3.6.2\\s[Build:\\s1695203293] client_platform=Windows client_talk_request=1791300000 client_talk_request_msg=one\\squestion client_is_priority_speaker=0",
        ));
        let marlin = &book.clients[&4];
        assert_eq!((marlin.version.as_str(), marlin.platform.as_str()), ("3.6.2 [Build: 1695203293]", "Windows"));
        assert!(marlin.talk_request && !marlin.is_priority_speaker);
        assert_eq!(marlin.talk_request_message, "one question");
        book.clients_updated(&Command::parse("notifyclientupdated clid=4 client_talk_request=0 client_talk_request_msg"));
        assert!(!book.clients[&4].talk_request && book.clients[&4].talk_request_message.is_empty());
    }

    #[test]
    fn channel_edit_and_move() {
        let mut book = book_from("channellist cid=1 cpid=0 channel_order=0 channel_name=A|cid=2 cpid=0 channel_order=1 channel_name=B");
        book.upsert_channels(&Command::parse(
            "notifychanneledited cid=2 reasonid=10 invokerid=1 channel_name=Renamed channel_codec_quality=9",
        ));
        assert_eq!(book.channels[&2].name, "Renamed");
        assert_eq!(book.channels[&2].codec_quality, 9);
        assert!(!book.channels[&2].description_known);
        book.upsert_channels(&Command::parse("notifychanneledited cid=2 channel_description=[b]rules[\\/b]\\nbe\\skind reasonid=9"));
        assert_eq!(book.channels[&2].description, "[b]rules[/b]\nbe kind");
        assert!(book.channels[&2].description_known && !book.channels[&1].description_known);
        book.description_changed(&Command::parse("notifychanneldescriptionchanged cid=2"));
        assert!(!book.channels[&2].description_known);
        book.upsert_channels(&Command::parse("notifychannelmoved cid=2 cpid=1 order=0 reasonid=1"));
        assert_eq!(book.channel_order(), vec![(1, 0), (2, 1)]);
        book.upsert_channels(&Command::parse(
            "notifychannelcreated cid=3 cpid=0 channel_name=New channel_order=1 invokerid=2",
        ));
        assert_eq!(book.channel_order(), vec![(1, 0), (2, 1), (3, 0)]);
    }

    #[test]
    fn server_info() {
        let mut s = ServerInfo::default();
        s.apply(&Command::parse(
            "initserver virtualserver_name=TeamSpeak\\s]I[\\sServer virtualserver_welcomemessage=Welcome virtualserver_platform=Linux virtualserver_version=3.13.7\\s[Build:\\s1655727713] virtualserver_maxclients=32 virtualserver_codec_encryption_mode=2 virtualserver_id=1 aclid=2",
        ));
        assert_eq!(s.name, "TeamSpeak ]I[ Server");
        assert_eq!(s.version, "3.13.7 [Build: 1655727713]");
        assert_eq!(s.max_clients, 32);
        assert_eq!(s.codec_encryption_mode, 2);
        assert_eq!(s.virtual_server_id, 1);
        s.apply(&Command::parse("notifyserveredited virtualserver_hostmessage=Mind\\sthe\\scoral virtualserver_hostmessage_mode=2"));
        assert_eq!((s.host_message.as_str(), s.host_message_mode), ("Mind the coral", 2));
    }
}
