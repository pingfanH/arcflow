//! External-control session authorization.

use core::fmt;

use arcflow_plugin_runtime::Capability;

use crate::{ClientHello, PROTOCOL_VERSION};

/// Policy used when accepting external-control clients.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayPolicy {
    allowed_capabilities: Vec<Capability>,
}

impl GatewayPolicy {
    /// Constructs a policy from the capabilities the user approved.
    #[must_use]
    pub fn new(allowed_capabilities: Vec<Capability>) -> Self {
        Self {
            allowed_capabilities,
        }
    }

    /// Constructs a local default policy with read-only device access.
    #[must_use]
    pub fn local_default() -> Self {
        Self::new(vec![Capability::DeviceRead, Capability::EventsSubscribe])
    }

    /// Returns the capabilities allowed by this policy.
    #[must_use]
    pub fn allowed_capabilities(&self) -> &[Capability] {
        &self.allowed_capabilities
    }

    /// Accepts a client hello and returns a granted session.
    pub fn accept(&self, hello: ClientHello) -> Result<ClientSession, SessionError> {
        if hello.protocol_version != PROTOCOL_VERSION {
            return Err(SessionError::UnsupportedProtocolVersion {
                requested: hello.protocol_version,
                supported: PROTOCOL_VERSION,
            });
        }

        if hello.client_name.trim().is_empty() {
            return Err(SessionError::BlankClientName);
        }

        for capability in &hello.requested_capabilities {
            if !self.allowed_capabilities.contains(capability) {
                return Err(SessionError::CapabilityDenied(*capability));
            }
        }

        Ok(ClientSession {
            client_name: hello.client_name,
            granted_capabilities: hello.requested_capabilities,
        })
    }
}

/// Granted external-control session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientSession {
    client_name: String,
    granted_capabilities: Vec<Capability>,
}

impl ClientSession {
    /// Returns the external client name.
    #[must_use]
    pub fn client_name(&self) -> &str {
        &self.client_name
    }

    /// Returns the capabilities granted to this session.
    #[must_use]
    pub fn granted_capabilities(&self) -> &[Capability] {
        &self.granted_capabilities
    }

    /// Returns whether the session has a capability.
    #[must_use]
    pub fn has_capability(&self, capability: Capability) -> bool {
        self.granted_capabilities.contains(&capability)
    }
}

/// Error returned when an external-control session cannot be accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    /// Client requested an unsupported protocol version.
    UnsupportedProtocolVersion {
        /// Requested protocol version.
        requested: u16,
        /// Supported protocol version.
        supported: u16,
    },
    /// Client name was blank.
    BlankClientName,
    /// Requested capability is not allowed by the policy.
    CapabilityDenied(Capability),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedProtocolVersion {
                requested,
                supported,
            } => write!(
                f,
                "external-control protocol {requested} is unsupported; expected {supported}"
            ),
            Self::BlankClientName => write!(f, "external-control client name must not be blank"),
            Self::CapabilityDenied(capability) => {
                write!(
                    f,
                    "external-control capability denied: {}",
                    capability.as_str()
                )
            }
        }
    }
}

impl std::error::Error for SessionError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_allowed_client() {
        let hello = ClientHello {
            client_name: "OBS ArcFlow Bridge".to_owned(),
            protocol_version: PROTOCOL_VERSION,
            requested_capabilities: vec![Capability::DeviceRead],
        };

        let session = GatewayPolicy::local_default().accept(hello).unwrap();

        assert_eq!(session.client_name(), "OBS ArcFlow Bridge");
        assert!(session.has_capability(Capability::DeviceRead));
    }

    #[test]
    fn denies_unapproved_capability() {
        let hello = ClientHello {
            client_name: "Unsafe Controller".to_owned(),
            protocol_version: PROTOCOL_VERSION,
            requested_capabilities: vec![Capability::WaveControl],
        };

        let error = GatewayPolicy::local_default().accept(hello).unwrap_err();

        assert_eq!(
            error,
            SessionError::CapabilityDenied(Capability::WaveControl)
        );
    }
}
