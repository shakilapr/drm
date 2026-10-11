//! Synthetic-publisher import hook (work-plan M10W1-E1-T1; router-core.md
//! I-RT4/I-RT9): the router deposits a received, header-stripped payload into
//! the local shared-memory channel through the ordinary [`Channel::publish`]
//! path — the same path a local publisher uses, with no router-only privilege.
//! The wire payload is copied into the channel's pool and the caller's slice is
//! released immediately, so no borrowed DRM sample outlives the ingress scope
//! (AGENTS.md #5 transient borrow lifecycle).
//! REQ: REQ-ROUTE-031, REQ-ROUTE-023.

use drm_core::channel::Channel;
use drm_types::Status;

use crate::data_path::ImportSink;

/// Import hook binding a single local topic's SHM channel as the synthetic
/// publisher's destination (router-core.md §Protocols *Disconnect isolation*).
/// A received frame's payload — already stripped of wire headers — is republished
/// verbatim, so a consumer reading this channel cannot distinguish imported from
/// local data (I-RT4). One `ChannelSink` binds one topic; a router serving
/// several imported topics holds one sink per topic.
/// REQ: REQ-ROUTE-031.
pub struct ChannelSink<'c> {
    channel: &'c Channel,
}

impl<'c> ChannelSink<'c> {
    /// Binds `channel` as the import destination.
    /// REQ: REQ-ROUTE-031.
    #[must_use]
    pub fn new(channel: &'c Channel) -> Self {
        Self { channel }
    }

    /// The bound channel.
    /// REQ: REQ-ROUTE-031.
    #[must_use]
    pub fn channel(&self) -> &'c Channel {
        self.channel
    }
}

impl ImportSink for ChannelSink<'_> {
    /// Republishes the native payload through the standard publish path; the
    /// channel copies it into its own pool, so the ingress borrow is released
    /// before returning (I-RT4, transient-borrow rule).
    /// REQ: REQ-ROUTE-031, REQ-ROUTE-023.
    ///
    /// # Errors
    /// The channel's own publish status (`Busy`, `QueueFull`, `PoolExhausted`,
    /// …); the caller counts it and keeps the forwarding tree running (I-RT16).
    fn deposit(
        &mut self,
        _topic: u32,
        _epoch: u64,
        _seq: u64,
        payload: &[u8],
    ) -> Result<(), Status> {
        self.channel.publish(payload).map(|_| ())
    }

    /// Bumps the imported publisher's stream epoch on a higher-epoch frame
    /// (`I-RT9`), reusing the publisher-lifecycle restart path: the prior run
    /// becomes unaddressable and consumers observe the epoch change.
    /// REQ: REQ-ROUTE-031.
    fn reset_epoch(&mut self, _topic: u32) -> Result<(), Status> {
        let _ = self.channel.restart_run();
        Ok(())
    }
}
