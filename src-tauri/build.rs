fn main() {
    // 声明 app 命令清单：为每个命令生成 allow-*/deny-* 权限。
    // 注意副作用：一旦存在 app ACL manifest，**所有** app 命令（含本地页面）都转为
    // ACL 管控，必须在 capabilities 里逐个 allow，否则本地页面调用也会被拒。
    // 这样做是为了让 capabilities/dsh-remote.json 能只给远程 dsh 源开放 zoom_ui 一个命令。
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new().commands(&[
                "get_shell_ui_state",
                "get_status",
                "restart_dsh",
                "get_recent_logs",
                "get_autostart",
                "set_autostart",
                "get_bootstrap_error",
                "is_first_launch",
                "zoom_ui",
                "get_shell_settings",
                "set_shell_settings",
                "preview_completion_sound",
                "list_skills",
                "list_import_sources",
                "import_skills",
                "set_skill_enabled",
                "delete_skill",
                "list_mcp_servers",
                "upsert_mcp_server",
                "set_mcp_enabled",
                "delete_mcp_server",
                "list_mcp_import_sources",
                "import_mcp_servers",
            ]),
        ),
    )
    .unwrap();
}
