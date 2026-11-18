// Kein Konsolenfenster im Release-Build
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // MCP-Server für Claude: ohne Fenster, Tray und Einzelinstanz, nur stdin/stdout (mcp.rs)
    if std::env::args().any(|a| a == "--mcp") {
        return helio_lib::mcp_stdio();
    }
    helio_lib::run()
}
