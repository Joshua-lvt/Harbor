// Prevents additional console window on Windows, keeps stdout on Linux.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use app_lib::run;

fn main() {
    run();
}
