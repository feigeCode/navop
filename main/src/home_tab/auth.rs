use super::team_permissions::RestoreFailureResolution;
use super::*;

impl HomePage {
    fn apply_authenticated_user(&mut self, user: &UserInfo, cx: &mut Context<Self>) {
        self.team_permissions.set_user_id(Some(user.id.clone()));
        self.current_user = Some(user.clone());
        GlobalCurrentUser::set_user(Some(user.clone()), cx);
        self.load_team_options(cx);
    }

    fn apply_restored_user(
        &mut self,
        requested_user_id: &str,
        user: &UserInfo,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.team_permissions.user_id() != Some(requested_user_id) {
            return false;
        }
        self.apply_authenticated_user(user, cx);
        cx.notify();
        if should_auto_onet_cloud_sync(cx, self.current_user.is_some()) {
            tracing::info!("会话已恢复且密钥已解锁，自动触发云同步");
            self.trigger_sync(cx);
        }
        true
    }

    fn reconcile_failed_session_restore(
        &mut self,
        requested_user_id: &str,
        persisted_user_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let resolution = self
            .team_permissions
            .reconcile_restore_failure(requested_user_id, persisted_user_id);
        if resolution != RestoreFailureResolution::IdentityChanged {
            return;
        }
        self.current_user = None;
        GlobalCurrentUser::set_user(None, cx);
        self.load_team_options(cx);
        cx.notify();
    }

    pub(super) fn clear_authenticated_state(&mut self) {
        self.current_user = None;
        self.team_permissions.clear();
    }

    /// 尝试从本地存储恢复会话
    pub(super) fn try_restore_session(&mut self, cx: &mut Context<Self>) {
        let Some(requested_user_id) = self.team_permissions.user_id().map(str::to_owned) else {
            return;
        };
        let auth = self.auth_service.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Some(user) = auth.try_restore_session().await else {
                let persisted_user_id = load_auth_data().map(|(_, _, user_id, _)| user_id);
                _ = this.update(cx, |this, cx| {
                    this.reconcile_failed_session_restore(
                        &requested_user_id,
                        persisted_user_id,
                        cx,
                    );
                });
                return;
            };

            let restored = this.update(cx, |this, cx| {
                this.apply_restored_user(&requested_user_id, &user, cx)
            });
            if !matches!(restored, Ok(true)) {
                return;
            }

            let subscription = auth.cloud_client().get_subscription().await.ok().flatten();
            _ = this.update(cx, |this, cx| {
                if this
                    .current_user
                    .as_ref()
                    .map(|current| current.id.as_str())
                    != Some(user.id.as_str())
                {
                    return;
                }
                let license_service = get_license_service(cx);
                if let Err(error) = license_service.update_from_subscription(user.id, subscription)
                {
                    tracing::warn!("更新 License 失败: {}", error);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 使用 OTP 验证码登录
    pub(super) fn verify_otp(&mut self, email: String, otp: String, cx: &mut Context<Self>) {
        self.logging_in = true;
        self.auth_error = None;
        cx.notify();

        let auth = self.auth_service.clone();

        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = auth.verify_otp(&email, &otp).await;

            // 如果登录成功，获取订阅信息
            let subscription = if result.is_ok() {
                auth.cloud_client().get_subscription().await.ok().flatten()
            } else {
                None
            };

            _ = this.update(cx, |this, cx| {
                this.logging_in = false;
                match result {
                    Ok(user) => {
                        this.apply_authenticated_user(&user, cx);

                        // 更新 License
                        let license_service = get_license_service(cx);
                        if let Err(e) =
                            license_service.update_from_subscription(user.id, subscription)
                        {
                            tracing::warn!("更新 License 失败: {}", e);
                        }

                        this.auth_error = None;
                        // 登录成功后，如果密钥已解锁，自动触发同步
                        if should_auto_onet_cloud_sync(cx, this.current_user.is_some()) {
                            tracing::info!("登录成功且密钥已解锁，自动触发云同步");
                            this.trigger_sync(cx);
                        }
                    }
                    Err(e) => {
                        tracing::error!("OTP 验证失败: {}", e);
                        this.auth_error = Some(e);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 显示登录对话框（OTP 模式）
    pub(crate) fn show_login_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity();
        show_auth_dialog(window, cx, view, |this, email, otp, cx| {
            this.verify_otp(email, otp, cx);
        });
    }
}
