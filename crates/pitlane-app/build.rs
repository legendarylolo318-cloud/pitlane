fn main() {
    // Fluent follows the system light/dark scheme through `Palette`.
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/app.slint", config).expect("slint compile");
}
