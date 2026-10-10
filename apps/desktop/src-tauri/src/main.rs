// Release builds on Windows are window apps: no console window opens behind them.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() { lute_and_tavern_lib::run() }
