fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut resource = winres::WindowsResource::new();
        resource.set_icon("assets/wincache.ico");
        resource.set("FileDescription", "WinCache Cleaner - limpeza de caches");
        resource.set("ProductName", "WinCache Cleaner");
        resource.compile().expect("Erro ao incorporar o icone no executavel");
    }
}
