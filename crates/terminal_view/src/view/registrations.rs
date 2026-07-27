use super::*;

impl TerminalView {
    pub(super) fn register_broadcast_input(&mut self, cx: &mut Context<Self>) {
        if self.broadcast_client_id.is_some() {
            return;
        }

        let label = {
            let terminal = self.terminal.read(cx);
            if !live_ssh_feature_supported(terminal.live_connection_kind()) {
                return;
            }
            let base = terminal
                .connection_name()
                .filter(|name| !name.is_empty())
                .or_else(|| (!terminal.title().is_empty()).then(|| terminal.title()))
                .unwrap_or("SSH Terminal");
            self.tab_index
                .map(|index| format!("{base}({index})"))
                .unwrap_or_else(|| base.to_string())
        };

        init_broadcast_input_registry(cx);
        let view = cx.entity().downgrade();
        let Some(registry) = broadcast_input_registry(cx) else {
            return;
        };
        let client_id = registry.update(cx, |registry, cx| registry.register(label, view, cx));
        self.broadcast_client_id = Some(client_id);
    }

    pub(super) fn unregister_broadcast_input(&mut self, cx: &mut Context<Self>) {
        let Some(client_id) = self.broadcast_client_id.take() else {
            return;
        };
        if let Some(registry) = broadcast_input_registry(cx) {
            registry.update(cx, |registry, cx| registry.unregister(client_id, cx));
        }
    }

    pub(super) fn broadcast_user_input(&self, data: &[u8], cx: &mut Context<Self>) {
        if !self.is_live_ssh_terminal(cx) {
            return;
        }
        let Some(client_id) = self.broadcast_client_id else {
            return;
        };
        let Some(registry) = broadcast_input_registry(cx) else {
            return;
        };
        let deliveries = registry.read(cx).deliveries_from(client_id, data);
        for (view, data) in deliveries {
            let _ = view.update(cx, |view, cx| {
                view.write_broadcast_input(data, cx);
            });
        }
    }

    pub(super) fn refresh_public_mcp_session(&self, cx: &mut Context<Self>) {
        let Some(registration) = &self.public_mcp_registration else {
            return;
        };
        registration.refresh(self.terminal.read(cx));
    }

    pub(super) fn unregister_public_mcp_session(&mut self, cx: &mut Context<Self>) {
        if let Some(registration) = self.public_mcp_registration.take() {
            registration.unregister(cx);
        }
    }
}
