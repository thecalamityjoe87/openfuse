//! Print the app icon as SVG (used when building the .deb package).
fn main() {
    print!("{}", openfuse::icons::icon_svg(false, false));
}
