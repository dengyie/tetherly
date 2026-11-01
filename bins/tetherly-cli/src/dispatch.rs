// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The `tetherly-cli` dispatch table: one arm per `Cmd` variant, each delegating to a handler.

use super::*;

pub(crate) async fn dispatch(cli: Cli) -> Result<()> {
    match cli.cmd {
        Cmd::Identity => {
            cmd_core::identity().await?;
        }
        Cmd::Pin {
            status,
            clear,
            json,
            ui_port,
        } => {
            cmd_core::pin(status, clear, json, ui_port).await?;
        }
        Cmd::Listen { port, pin, name } => {
            cmd_core::listen(port, pin, name).await?;
        }
        Cmd::Dial {
            addr,
            peer,
            pin,
            ui_port,
        } => {
            cmd_core::dial(addr, peer, pin, ui_port).await?;
        }
        Cmd::Discover {
            rescan,
            json,
            ui_port,
        } => {
            cmd_core::discover(rescan, json, ui_port).await?;
        }
        Cmd::Notify {
            title,
            body,
            app_id,
            peer,
            ui_port,
        } => {
            cmd_notify::notify(title, body, app_id, peer, ui_port).await?;
        }
        Cmd::Send {
            paths,
            name,
            peer,
            tag,
            group,
            device_type,
            all,
            json,
            watch,
            ui_port,
        } => {
            cmd_files::send(
                paths,
                name,
                peer,
                tag,
                group,
                device_type,
                all,
                json,
                watch,
                ui_port,
            )
            .await?;
        }
        Cmd::Resume { json, ui_port } => {
            cmd_files::resume(json, ui_port).await?;
        }
        Cmd::Cancel { id, ui_port } => {
            cmd_files::cancel(id, ui_port).await?;
        }
        Cmd::Transfers {
            id,
            active,
            terminal,
            direction,
            json,
            ui_port,
        } => {
            cmd_files::transfers(id, active, terminal, direction, json, ui_port).await?;
        }
        Cmd::ClearTransfers { ui_port } => {
            cmd_files::clear_transfers(ui_port).await?;
        }
        Cmd::Offers {
            peer,
            id,
            json,
            ui_port,
        } => {
            cmd_files::offers(peer, id, json, ui_port).await?;
        }
        Cmd::Accept {
            id,
            all,
            peer,
            json,
            ui_port,
        } => {
            cmd_files::accept(id, all, peer, json, ui_port).await?;
        }
        Cmd::Reject {
            id,
            all,
            peer,
            json,
            ui_port,
        } => {
            cmd_files::reject(id, all, peer, json, ui_port).await?;
        }
        Cmd::AutoAccept {
            enable,
            disable,
            group,
            tag,
            device_type,
            max_size,
            clear_filters,
            json,
            ui_port,
        } => {
            cmd_files::auto_accept(
                enable,
                disable,
                group,
                tag,
                device_type,
                max_size,
                clear_filters,
                json,
                ui_port,
            )
            .await?;
        }
        Cmd::Unpair { device_id, ui_port } => {
            cmd_peers::unpair(device_id, ui_port).await?;
        }
        Cmd::Clip { action } => {
            cmd_clip::clip(action).await?;
        }
        Cmd::Handoff { action } => {
            cmd_handoff::handoff(action).await?;
        }
        Cmd::Media { action } => {
            cmd_media::media(action).await?;
        }
        Cmd::Power { action } => {
            cmd_power::power(action).await?;
        }
        Cmd::Locate { action } => {
            cmd_locate::locate(action).await?;
        }
        Cmd::Action { action } => {
            cmd_action::action(action).await?;
        }
        Cmd::Focus { action } => {
            cmd_focus::focus(action).await?;
        }
        Cmd::Topology { action } => {
            cmd_topology::topology(action).await?;
        }
        Cmd::Candidates {
            peer,
            app_id,
            has_otp,
            json,
            ui_port,
        } => {
            cmd_notify::candidates(peer, app_id, has_otp, json, ui_port).await?;
        }
        Cmd::Otp { id, copy, ui_port } => {
            cmd_notify::otp(id, copy, ui_port).await?;
        }
        Cmd::Dismiss {
            id,
            peer,
            app_id,
            all,
            json,
            ui_port,
        } => {
            cmd_notify::dismiss(id, peer, app_id, all, json, ui_port).await?;
        }
        Cmd::Reply { id, text, ui_port } => {
            cmd_notify::reply(id, text, ui_port).await?;
        }
        Cmd::Replies { json, ui_port } => {
            cmd_notify::replies(json, ui_port).await?;
        }
        Cmd::TakeReply { id, ui_port } => {
            cmd_notify::take_reply(id, ui_port).await?;
        }
        Cmd::Screen { action } => {
            cmd_screen::screen(action).await?;
        }
        Cmd::Input { action } => {
            cmd_input::input(action).await?;
        }
        Cmd::Display { action } => {
            cmd_display::display(action).await?;
        }
        Cmd::Audio { action } => {
            cmd_audio::audio(action).await?;
        }
        Cmd::Camera { action } => {
            cmd_camera::camera(action).await?;
        }
        Cmd::Mic { action } => {
            cmd_mic::mic(action).await?;
        }
        Cmd::Bench { action } => {
            cmd_bench::bench(action).await?;
        }
        Cmd::Recommend {
            peer,
            json,
            ui_port,
        } => {
            cmd_bench::recommend(peer, json, ui_port).await?;
        }
        Cmd::Peers {
            trusted,
            ping,
            peer,
            count,
            json,
            ui_port,
        } => {
            cmd_peers::peers(trusted, ping, peer, count, json, ui_port).await?;
        }
        Cmd::Alias {
            peer,
            alias,
            ui_port,
        } => {
            cmd_peers::alias(peer, alias, ui_port).await?;
        }
        Cmd::Routes { json, ui_port } => {
            cmd_peers::routes(json, ui_port).await?;
        }
        Cmd::Route { action } => {
            cmd_peers::route(action).await?;
        }
        Cmd::Net {
            refresh,
            interfaces,
            json,
            ui_port,
        } => {
            cmd_peers::net(refresh, interfaces, json, ui_port).await?;
        }
        Cmd::Ancs { action } => {
            cmd_ancs::ancs(action).await?;
        }
        Cmd::Storage { action } => {
            cmd_files::storage(action).await?;
        }
        Cmd::Inbox { action } => {
            cmd_files::inbox(action).await?;
        }
        Cmd::Link {
            action,
            json,
            ui_port,
        } => {
            cmd_peers::link(action, json, ui_port).await?;
        }
        Cmd::Audit {
            action,
            category,
            severity,
            limit,
            since,
            watch,
            interval,
            clear,
            json,
            ui_port,
        } => {
            cmd_audit::audit(
                action, category, severity, limit, since, watch, interval, clear, json, ui_port,
            )
            .await?;
        }
        Cmd::Message {
            action,
            peer,
            tag,
            group,
            device_type,
            all,
            text,
            limit,
            unread,
            mark_read,
            clear,
            watch,
            json,
            ui_port,
        } => {
            cmd_messages::message(
                action,
                peer,
                tag,
                group,
                device_type,
                all,
                text,
                limit,
                unread,
                mark_read,
                clear,
                watch,
                json,
                ui_port,
            )
            .await?;
        }
        Cmd::Outbox {
            action,
            peer,
            status,
            limit,
            clear,
            resend,
            watch,
            json,
            ui_port,
        } => {
            cmd_files::outbox(
                action, peer, status, limit, clear, resend, watch, json, ui_port,
            )
            .await?;
        }
        Cmd::Banner { action } => {
            cmd_notify::banner(action).await?;
        }
        Cmd::Profile {
            action,
            tag,
            group,
            device_type,
            json,
            ui_port,
        } => {
            cmd_profile::profile(action, tag, group, device_type, json, ui_port).await?;
        }
        Cmd::Tag { action } => {
            cmd_profile::tag(action).await?;
        }
        Cmd::Group { action } => {
            cmd_profile::group(action).await?;
        }
        Cmd::Status {
            watch,
            interval,
            json,
            ui_port,
        } => {
            cmd_core::status(watch, interval, json, ui_port).await?;
        }
        Cmd::Health { json, ui_port } => {
            cmd_core::health(json, ui_port).await?;
        }
        Cmd::Sys {
            watch,
            interval,
            json,
            ui_port,
        } => {
            cmd_core::sys(watch, interval, json, ui_port).await?;
        }
        Cmd::Config {
            name,
            advertise,
            reconnect,
            json,
            ui_port,
        } => {
            cmd_core::config(name, advertise, reconnect, json, ui_port).await?;
        }
        Cmd::Ping {
            peer,
            count,
            json,
            ui_port,
        } => {
            cmd_peers::ping(peer, count, json, ui_port).await?;
        }
        Cmd::Stop { ui_port } => {
            cmd_core::stop(ui_port).await?;
        }
        Cmd::Events {
            count,
            timeout_secs,
            json,
            ui_port,
        } => {
            cmd_core::events(count, timeout_secs, json, ui_port).await?;
        }
        Cmd::Metrics {
            reset,
            json,
            ui_port,
        }
        | Cmd::Traffic {
            reset,
            json,
            ui_port,
        } => {
            cmd_core::metrics(reset, json, ui_port).await?;
        }
        Cmd::Caps {
            peer,
            enable,
            disable,
            json,
            ui_port,
        } => {
            cmd_core::caps(peer, enable, disable, json, ui_port).await?;
        }
        Cmd::Connect {
            peer,
            addr,
            json,
            ui_port,
        } => {
            cmd_peers::connect(peer, addr, json, ui_port).await?;
        }
        Cmd::Disconnect {
            peer,
            json,
            ui_port,
        } => {
            cmd_peers::disconnect(peer, json, ui_port).await?;
        }
        Cmd::Block {
            peer,
            reason,
            json,
            ui_port,
        } => {
            cmd_peers::block(peer, reason, json, ui_port).await?;
        }
        Cmd::Unblock {
            peer,
            json,
            ui_port,
        } => {
            cmd_peers::unblock(peer, json, ui_port).await?;
        }
        Cmd::Blocked { json, ui_port } => {
            cmd_peers::blocked(json, ui_port).await?;
        }
    }
    Ok(())
}
