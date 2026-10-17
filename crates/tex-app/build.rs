fn main() {
    embed_resource::compile("assets/tex.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("icon resource failed to compile");
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_bundled_translations("ui/translations")
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/app.slint", config).expect("Slint UI failed to compile");
}
