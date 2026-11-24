// Pas de console en version finale sous Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    chud_copy_lib::run();
}
