fn main() {
    slint_build::compile("ui/app_window.slint").expect("Failed to compile Slint UI definition");

    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/sane-rec.ico");
        res.compile().unwrap();
    }
}
