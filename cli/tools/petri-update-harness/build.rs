fn main() {
    // This separate development executable is never packaged or exported.
    println!("cargo:rustc-env=PETRI_UPDATE_CHANNEL=preview");
}
