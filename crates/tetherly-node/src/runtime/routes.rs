// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Route table, peer aliases, profiles, tags and groups.

use super::*;
use crate::events::UiEvent;
use crate::store::{DeviceType, PeerProfile, ProfileTable, TargetSelector};
use std::net::SocketAddr;
use tetherly_core::TrustStore;
use tetherly_core::{CoreError, DeviceId};

/// Query report for peer profiles.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerProfilesReport {
    pub ok: bool,
    pub total: usize,
    pub profiles: Vec<PeerProfile>,
}

/// Report after updating a peer profile.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerProfileUpdateReport {
    pub ok: bool,
    pub device_id: String,
    pub profile: PeerProfile,
    pub message: String,
}

/// Report after modifying peer tags.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerTagsReport {
    pub ok: bool,
    pub device_id: String,
    pub tags: Vec<String>,
    pub message: String,
}

/// Report after setting or clearing a peer group.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerGroupReport {
    pub ok: bool,
    pub device_id: String,
    pub group: Option<String>,
    pub message: String,
}

/// Physical addressing route entry with enriched peer status metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RouteEntryDto {
    pub device_id: String,
    pub addr: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub online: bool,
    pub trusted: bool,
}

/// Comprehensive report of cached physical addressing routes.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RoutesReport {
    pub ok: bool,
    pub count: usize,
    pub routes: Vec<RouteEntryDto>,
}

/// Result report from adding or removing a physical addressing route.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RouteMutateReport {
    pub ok: bool,
    pub device_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addr: Option<String>,
    pub action: String,
}

impl Node {
    /// Retrieve the local friendly alias for a trusted peer device, if set and not revoked.
    pub fn peer_alias(&self, device_id: &DeviceId) -> Option<String> {
        let store = self.shared.trust.lock().expect("trust");
        store
            .peers
            .get(device_id)
            .filter(|p| !p.revoked && !p.alias.is_empty())
            .map(|p| p.alias.clone())
    }

    /// Update the local friendly alias for a trusted peer device.
    /// Validates length and characters, persists to trusted.json, and emits `UiEvent::PeerAliasChanged`.
    pub fn set_peer_alias(&self, device_id: &DeviceId, alias: &str) -> Result<bool, CoreError> {
        let trimmed = alias.trim();
        if trimmed.is_empty() {
            return Err(CoreError::InvalidAlias("alias cannot be empty".to_string()));
        }
        if trimmed.len() > 64 {
            return Err(CoreError::InvalidAlias(
                "alias exceeds maximum length of 64 bytes".to_string(),
            ));
        }
        if trimmed.chars().any(|c| c.is_control()) {
            return Err(CoreError::InvalidAlias(
                "alias contains disallowed control characters".to_string(),
            ));
        }

        let updated = {
            let mut store = self.shared.trust.lock().expect("trust");
            store.update_alias(device_id, trimmed)?
        };

        if updated {
            self.persist_trust();
            self.emit(UiEvent::PeerAliasChanged {
                device_id: device_id.to_string(),
                alias: trimmed.to_string(),
            });
        }

        Ok(updated)
    }

    /// Retrieve the metadata profile for a device. Defaults to empty profile if unrecorded.
    pub fn peer_profile(&self, device_id: &DeviceId) -> PeerProfile {
        let id_str = device_id.to_string();
        let table = self.shared.profiles.lock().expect("profiles");
        table
            .profiles
            .iter()
            .find(|p| p.device_id == id_str)
            .cloned()
            .unwrap_or_else(|| PeerProfile {
                device_id: id_str,
                device_type: DeviceType::Other,
                tags: Vec::new(),
                group: None,
                notes: None,
                updated_ms: 0,
            })
    }

    /// Query and filter peer profiles by tag, group, or device type.
    pub fn peer_profiles(
        &self,
        tag: Option<&str>,
        group: Option<&str>,
        device_type: Option<DeviceType>,
    ) -> PeerProfilesReport {
        let table = self.shared.profiles.lock().expect("profiles");
        let mut profiles = table.profiles.clone();

        if let Some(t) = tag {
            let t_clean = t.trim().to_ascii_lowercase();
            profiles.retain(|p| {
                p.tags
                    .iter()
                    .any(|tag_item| tag_item.to_ascii_lowercase() == t_clean)
            });
        }
        if let Some(g) = group {
            let g_clean = g.trim().to_ascii_lowercase();
            profiles.retain(|p| {
                p.group
                    .as_deref()
                    .map(|s| s.to_ascii_lowercase())
                    .as_deref()
                    == Some(&g_clean)
            });
        }
        if let Some(dt) = device_type {
            profiles.retain(|p| p.device_type == dt);
        }

        PeerProfilesReport {
            ok: true,
            total: profiles.len(),
            profiles,
        }
    }

    pub(super) fn persist_profiles(&self, table: &ProfileTable) -> Result<(), CoreError> {
        crate::store::save_profiles(&self.shared.cfg.data_dir, table)
            .map_err(|e| CoreError::File(format!("failed to save profiles.json: {e}")))
    }

    /// Update a peer's profile attributes (type, tags, group, notes).
    pub fn update_peer_profile(
        &self,
        target: &str,
        device_type: Option<DeviceType>,
        tags: Option<Vec<String>>,
        group: Option<Option<String>>,
        notes: Option<Option<String>>,
    ) -> Result<PeerProfileUpdateReport, CoreError> {
        let device_id = self.resolve_peer(target)?;
        let device_id_str = device_id.to_string();

        let validated_tags = if let Some(tag_list) = tags {
            if tag_list.len() > 16 {
                return Err(CoreError::InvalidProfile(
                    "a device can have at most 16 tags".to_string(),
                ));
            }
            let mut set = Vec::new();
            for t in tag_list {
                let val = validate_tag(&t)?;
                if !set.contains(&val) {
                    set.push(val);
                }
            }
            Some(set)
        } else {
            None
        };

        let validated_group = if let Some(g) = group {
            match g {
                Some(ref g_str) => Some(validate_group(g_str)?),
                None => Some(None),
            }
        } else {
            None
        };

        let validated_notes = if let Some(n) = notes {
            match n {
                Some(ref n_str) => Some(validate_notes(n_str)?),
                None => Some(None),
            }
        } else {
            None
        };

        let now = self.shared.clock.unix_ms();

        let updated_profile = {
            let mut table = self.shared.profiles.lock().expect("profiles");
            let pos = table
                .profiles
                .iter()
                .position(|p| p.device_id == device_id_str);
            let profile = if let Some(idx) = pos {
                let p = &mut table.profiles[idx];
                if let Some(dt) = device_type {
                    p.device_type = dt;
                }
                if let Some(t) = validated_tags {
                    p.tags = t;
                }
                if let Some(g) = validated_group {
                    p.group = g;
                }
                if let Some(n) = validated_notes {
                    p.notes = n;
                }
                p.updated_ms = now;
                p.clone()
            } else {
                let p = PeerProfile {
                    device_id: device_id_str.clone(),
                    device_type: device_type.unwrap_or_default(),
                    tags: validated_tags.unwrap_or_default(),
                    group: validated_group.flatten(),
                    notes: validated_notes.flatten(),
                    updated_ms: now,
                };
                table.profiles.push(p.clone());
                p
            };
            self.persist_profiles(&table)?;
            profile
        };

        // Zero-Plaintext event broadcast: strictly never broadcast notes!
        self.emit(UiEvent::peer_profile_updated(
            &device_id_str,
            updated_profile.device_type.as_str(),
            updated_profile.tags.clone(),
            updated_profile.group.clone(),
            updated_profile.updated_ms,
        ));

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "profile_updated",
            Some(device_id_str.clone()),
            format!(
                "profile updated for peer '{}': type={}, tags_count={}, group={:?}",
                device_id_str,
                updated_profile.device_type.as_str(),
                updated_profile.tags.len(),
                updated_profile.group
            ),
            true,
        );

        Ok(PeerProfileUpdateReport {
            ok: true,
            device_id: device_id_str,
            profile: updated_profile,
            message: "peer profile updated successfully".to_string(),
        })
    }

    /// Add a tag to a peer's profile.
    pub fn add_peer_tag(&self, target: &str, tag: &str) -> Result<PeerTagsReport, CoreError> {
        let clean_tag = validate_tag(tag)?;
        let device_id = self.resolve_peer(target)?;
        let device_id_str = device_id.to_string();
        let now = self.shared.clock.unix_ms();

        let (tags, dt_str, grp) = {
            let mut table = self.shared.profiles.lock().expect("profiles");
            let pos = table
                .profiles
                .iter()
                .position(|p| p.device_id == device_id_str);
            let (tags, dt_str, grp) = if let Some(idx) = pos {
                let p = &mut table.profiles[idx];
                if !p.tags.iter().any(|t| t.eq_ignore_ascii_case(&clean_tag)) {
                    if p.tags.len() >= 16 {
                        return Err(CoreError::InvalidProfile(
                            "maximum 16 tags exceeded".to_string(),
                        ));
                    }
                    p.tags.push(clean_tag.clone());
                    p.updated_ms = now;
                }
                (p.tags.clone(), p.device_type.as_str(), p.group.clone())
            } else {
                let p = PeerProfile {
                    device_id: device_id_str.clone(),
                    device_type: DeviceType::Other,
                    tags: vec![clean_tag.clone()],
                    group: None,
                    notes: None,
                    updated_ms: now,
                };
                table.profiles.push(p);
                (vec![clean_tag.clone()], "other", None)
            };
            self.persist_profiles(&table)?;
            (tags, dt_str, grp)
        };

        self.emit(UiEvent::peer_profile_updated(
            &device_id_str,
            dt_str,
            tags.clone(),
            grp,
            now,
        ));

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "tag_added",
            Some(device_id_str.clone()),
            format!("added tag '{clean_tag}' to peer '{device_id_str}'"),
            true,
        );

        Ok(PeerTagsReport {
            ok: true,
            device_id: device_id_str,
            tags,
            message: format!("tag '{clean_tag}' added successfully"),
        })
    }

    /// Remove a tag from a peer's profile.
    pub fn remove_peer_tag(&self, target: &str, tag: &str) -> Result<PeerTagsReport, CoreError> {
        let clean_tag = tag.trim().to_ascii_lowercase();
        let device_id = self.resolve_peer(target)?;
        let device_id_str = device_id.to_string();
        let now = self.shared.clock.unix_ms();

        let (tags, dt_str, grp) = {
            let mut table = self.shared.profiles.lock().expect("profiles");
            let pos = table
                .profiles
                .iter()
                .position(|p| p.device_id == device_id_str);
            let (tags, dt_str, grp) = if let Some(idx) = pos {
                let p = &mut table.profiles[idx];
                p.tags.retain(|t| !t.eq_ignore_ascii_case(&clean_tag));
                p.updated_ms = now;
                (p.tags.clone(), p.device_type.as_str(), p.group.clone())
            } else {
                (Vec::new(), "other", None)
            };
            self.persist_profiles(&table)?;
            (tags, dt_str, grp)
        };

        self.emit(UiEvent::peer_profile_updated(
            &device_id_str,
            dt_str,
            tags.clone(),
            grp,
            now,
        ));

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "tag_removed",
            Some(device_id_str.clone()),
            format!("removed tag '{clean_tag}' from peer '{device_id_str}'"),
            true,
        );

        Ok(PeerTagsReport {
            ok: true,
            device_id: device_id_str,
            tags,
            message: format!("tag '{clean_tag}' removed successfully"),
        })
    }

    /// Assign or clear a peer's group.
    pub fn set_peer_group(
        &self,
        target: &str,
        group: Option<&str>,
    ) -> Result<PeerGroupReport, CoreError> {
        let validated_group = match group {
            Some(g) => validate_group(g)?,
            None => None,
        };
        let device_id = self.resolve_peer(target)?;
        let device_id_str = device_id.to_string();
        let now = self.shared.clock.unix_ms();

        let (group_res, dt_str, tags) = {
            let mut table = self.shared.profiles.lock().expect("profiles");
            let pos = table
                .profiles
                .iter()
                .position(|p| p.device_id == device_id_str);
            let (grp, dt_str, tags_res) = if let Some(idx) = pos {
                let p = &mut table.profiles[idx];
                p.group = validated_group;
                p.updated_ms = now;
                (p.group.clone(), p.device_type.as_str(), p.tags.clone())
            } else {
                let p = PeerProfile {
                    device_id: device_id_str.clone(),
                    device_type: DeviceType::Other,
                    tags: Vec::new(),
                    group: validated_group.clone(),
                    notes: None,
                    updated_ms: now,
                };
                table.profiles.push(p);
                (validated_group, "other", Vec::new())
            };
            self.persist_profiles(&table)?;
            (grp, dt_str, tags_res)
        };

        self.emit(UiEvent::peer_profile_updated(
            &device_id_str,
            dt_str,
            tags,
            group_res.clone(),
            now,
        ));

        let action_desc = match &group_res {
            Some(g) => format!("set group to '{g}' for peer '{device_id_str}'"),
            None => format!("cleared group for peer '{device_id_str}'"),
        };

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "group_changed",
            Some(device_id_str.clone()),
            action_desc,
            true,
        );

        Ok(PeerGroupReport {
            ok: true,
            device_id: device_id_str,
            group: group_res,
            message: "peer group updated successfully".to_string(),
        })
    }

    /// Resolve all device IDs that match a given tag.
    pub fn resolve_peers_by_tag(&self, tag: &str) -> Vec<DeviceId> {
        let t_clean = tag.trim().to_ascii_lowercase();
        let table = self.shared.profiles.lock().expect("profiles");
        table
            .profiles
            .iter()
            .filter(|p| {
                p.tags
                    .iter()
                    .any(|item| item.to_ascii_lowercase() == t_clean)
            })
            .filter_map(|p| DeviceId::parse(&p.device_id).ok())
            .collect()
    }

    /// Resolve all device IDs that match a given group.
    pub fn resolve_peers_by_group(&self, group: &str) -> Vec<DeviceId> {
        let g_clean = group.trim().to_ascii_lowercase();
        let table = self.shared.profiles.lock().expect("profiles");
        table
            .profiles
            .iter()
            .filter(|p| {
                p.group
                    .as_deref()
                    .map(|s| s.to_ascii_lowercase())
                    .as_deref()
                    == Some(&g_clean)
            })
            .filter_map(|p| DeviceId::parse(&p.device_id).ok())
            .collect()
    }

    /// Resolve all device IDs that match a given device type.
    pub fn resolve_peers_by_type(&self, device_type: DeviceType) -> Vec<DeviceId> {
        let table = self.shared.profiles.lock().expect("profiles");
        table
            .profiles
            .iter()
            .filter(|p| p.device_type == device_type)
            .filter_map(|p| DeviceId::parse(&p.device_id).ok())
            .collect()
    }

    /// Resolve all candidate target peer device IDs matched by a `TargetSelector`.
    /// Preserves deduplication order and ensures at least one target candidate was found.
    pub fn resolve_dispatch_targets(
        &self,
        selector: &TargetSelector,
    ) -> Result<Vec<DeviceId>, CoreError> {
        let targets = match selector {
            TargetSelector::Peer(p) => {
                let id = self.resolve_peer(p)?;
                vec![id]
            }
            TargetSelector::Tag(t) => self.resolve_peers_by_tag(t),
            TargetSelector::Group(g) => self.resolve_peers_by_group(g),
            TargetSelector::Type(dt) => self.resolve_peers_by_type(*dt),
            TargetSelector::AllOnline => {
                let live = self.live_peers();
                live.into_iter().map(|p| p.device_id).collect()
            }
        };

        let mut unique = Vec::new();
        for id in targets {
            if !unique.contains(&id) {
                unique.push(id);
            }
        }

        if unique.is_empty() {
            return Err(CoreError::NoTargetsMatched(selector.as_label()));
        }

        Ok(unique)
    }

    pub fn routes(&self) -> Vec<crate::store::Route> {
        crate::store::load_routes(&self.shared.cfg.data_dir).routes
    }

    pub fn routes_report(&self) -> RoutesReport {
        let routes = crate::store::load_routes(&self.shared.cfg.data_dir).routes;
        let live = self.live_peers();
        let trusted = self.trusted();

        let entries = routes
            .into_iter()
            .map(|r| {
                let matched_live = live.iter().find(|l| l.device_id.as_str() == r.device_id);
                let matched_trusted = trusted.iter().find(|t| t.device_id.as_str() == r.device_id);

                let online = matched_live.is_some();
                let is_trusted = matched_trusted.map(|t| !t.revoked).unwrap_or(false);
                let alias = matched_trusted.and_then(|t| {
                    if t.alias.is_empty() {
                        None
                    } else {
                        Some(t.alias.clone())
                    }
                });
                let name = matched_live.map(|l| l.name.clone());

                RouteEntryDto {
                    device_id: r.device_id,
                    addr: r.addr,
                    alias,
                    name,
                    online,
                    trusted: is_trusted,
                }
            })
            .collect::<Vec<_>>();

        RoutesReport {
            ok: true,
            count: entries.len(),
            routes: entries,
        }
    }

    pub fn add_route(&self, peer_spec: &str, addr: &str) -> Result<RouteMutateReport, CoreError> {
        let addr_trimmed = addr.trim();
        let socket_addr: SocketAddr = addr_trimmed.parse().map_err(|e| {
            CoreError::InvalidRoute(format!("invalid socket address '{addr_trimmed}': {e}"))
        })?;
        if socket_addr.port() == 0 {
            return Err(CoreError::InvalidRoute("port cannot be 0".to_string()));
        }

        let peer_trimmed = peer_spec.trim();
        if peer_trimmed.is_empty() {
            return Err(CoreError::InvalidRoute(
                "peer identifier cannot be empty".to_string(),
            ));
        }
        let device_id = self.resolve_peer(peer_trimmed)?;
        let is_update = crate::store::remember_route(
            &self.shared.cfg.data_dir,
            device_id.as_str(),
            &socket_addr.to_string(),
        )
        .map_err(|e| CoreError::File(e.to_string()))?;

        let action = if is_update { "updated" } else { "added" };
        self.emit(UiEvent::RouteChanged {
            device_id: device_id.to_string(),
            addr: socket_addr.to_string(),
            action: action.to_string(),
        });
        self.record_audit(
            AuditCategory::Route,
            AuditSeverity::Info,
            format!("route_{action}"),
            Some(device_id.to_string()),
            format!("route {action} for peer '{device_id}' -> {socket_addr}"),
            true,
        );
        Ok(RouteMutateReport {
            ok: true,
            device_id: device_id.to_string(),
            addr: Some(socket_addr.to_string()),
            action: action.to_string(),
        })
    }

    pub fn remove_route(&self, peer_spec: &str) -> Result<RouteMutateReport, CoreError> {
        let peer_trimmed = peer_spec.trim();
        if peer_trimmed.is_empty() {
            return Err(CoreError::InvalidRoute(
                "peer identifier cannot be empty".to_string(),
            ));
        }
        let target_device_id = match self.resolve_peer(peer_trimmed) {
            Ok(id) => id.to_string(),
            Err(_) => {
                let routes = crate::store::load_routes(&self.shared.cfg.data_dir).routes;
                if let Some(r) = routes.iter().find(|r| r.device_id == peer_trimmed) {
                    r.device_id.clone()
                } else {
                    return Err(CoreError::PeerNotFound(format!(
                        "no route matching peer '{peer_trimmed}' found"
                    )));
                }
            }
        };

        let removed = crate::store::forget_route(&self.shared.cfg.data_dir, &target_device_id)
            .map_err(|e| CoreError::File(e.to_string()))?;
        if !removed {
            return Err(CoreError::PeerNotFound(format!(
                "no route found for peer '{target_device_id}'"
            )));
        }
        self.emit(UiEvent::RouteChanged {
            device_id: target_device_id.clone(),
            addr: String::new(),
            action: "removed".to_string(),
        });
        self.record_audit(
            AuditCategory::Route,
            AuditSeverity::Notice,
            "route_removed",
            Some(target_device_id.clone()),
            format!("route removed for peer '{target_device_id}'"),
            true,
        );
        Ok(RouteMutateReport {
            ok: true,
            device_id: target_device_id,
            addr: None,
            action: "removed".to_string(),
        })
    }
}
