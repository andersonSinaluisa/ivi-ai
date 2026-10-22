//! Layout de topics v1: `jarvis/v1/devices/{deviceId}/{up|down}/{channel}`.

use uuid::Uuid;

pub const PREFIX: &str = "jarvis/v1/devices/";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Tasks,
    Notifications,
    Control,
    Requests,
    Presence,
    Events,
}

impl Channel {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tasks => "tasks",
            Self::Notifications => "notifications",
            Self::Control => "control",
            Self::Requests => "requests",
            Self::Presence => "presence",
            Self::Events => "events",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "tasks" => Self::Tasks,
            "notifications" => Self::Notifications,
            "control" => Self::Control,
            "requests" => Self::Requests,
            "presence" => Self::Presence,
            "events" => Self::Events,
            _ => return None,
        })
    }
}

/// Topics de un device concreto. Nunca se construyen topics con strings sueltos.
#[derive(Debug, Clone)]
pub struct Topics {
    device_id: Uuid,
}

impl Topics {
    #[must_use]
    pub const fn new(device_id: Uuid) -> Self {
        Self { device_id }
    }

    /// Filtro único de suscripción del device: `.../{self}/down/#`.
    #[must_use]
    pub fn subscription(&self) -> String {
        format!("{PREFIX}{}/down/#", self.device_id.hyphenated())
    }

    #[must_use]
    pub fn up(&self, channel: Channel) -> String {
        format!("{PREFIX}{}/up/{}", self.device_id.hyphenated(), channel.as_str())
    }

    #[must_use]
    pub fn down(&self, channel: Channel) -> String {
        format!("{PREFIX}{}/down/{}", self.device_id.hyphenated(), channel.as_str())
    }

    /// Devuelve el canal si el topic es un `down/*` dirigido a este device; `None` en cualquier otro caso.
    #[must_use]
    pub fn parse_own_down(&self, topic: &str) -> Option<Channel> {
        let (device, direction, channel) = parse(topic)?;
        (device == self.device_id && direction == Direction::Down).then_some(channel)
    }
}

/// Descompone un topic v1. Exige UUID en minúsculas con guiones, como el Cloud.
#[must_use]
pub fn parse(topic: &str) -> Option<(Uuid, Direction, Channel)> {
    let rest = topic.strip_prefix(PREFIX)?;
    let mut parts = rest.split('/');
    let device = parts.next()?;
    let direction = parts.next()?;
    let channel = parts.next()?;
    if parts.next().is_some() || device.len() != 36 || device.chars().any(|c| c.is_ascii_uppercase()) {
        return None;
    }
    let device_id = Uuid::try_parse(device).ok()?;
    let direction = match direction {
        "up" => Direction::Up,
        "down" => Direction::Down,
        _ => return None,
    };
    Some((device_id, direction, Channel::parse(channel)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> Uuid {
        Uuid::parse_str("0192a6f0-2222-7aaa-8bbb-000000000002").expect("valid uuid")
    }

    #[test]
    fn builds_topics_like_the_cloud() {
        let t = Topics::new(device());
        assert_eq!(t.subscription(), "jarvis/v1/devices/0192a6f0-2222-7aaa-8bbb-000000000002/down/#");
        assert_eq!(t.up(Channel::Tasks), "jarvis/v1/devices/0192a6f0-2222-7aaa-8bbb-000000000002/up/tasks");
        assert_eq!(t.up(Channel::Presence), "jarvis/v1/devices/0192a6f0-2222-7aaa-8bbb-000000000002/up/presence");
    }

    #[test]
    fn accepts_only_own_down_topics() {
        let t = Topics::new(device());
        assert_eq!(t.parse_own_down(&t.down(Channel::Tasks)), Some(Channel::Tasks));
        assert_eq!(t.parse_own_down(&t.down(Channel::Control)), Some(Channel::Control));
        assert_eq!(t.parse_own_down(&t.up(Channel::Tasks)), None);
        let other = Topics::new(Uuid::nil());
        assert_eq!(t.parse_own_down(&other.down(Channel::Tasks)), None);
        assert_eq!(t.parse_own_down("jarvis/v1/devices/0192A6F0-2222-7AAA-8BBB-000000000002/down/tasks"), None);
        assert_eq!(t.parse_own_down("jarvis/v1/devices/0192a6f0-2222-7aaa-8bbb-000000000002/down/tasks/x"), None);
        assert_eq!(t.parse_own_down("jarvis/v1/devices/0192a6f0-2222-7aaa-8bbb-000000000002/down/other"), None);
    }
}
