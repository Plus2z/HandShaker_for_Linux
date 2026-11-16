fn main() -> std::io::Result<()> {
    println!("cargo:rerun-if-changed=proto/handshaker.proto");
    prost_build::Config::new()
        .compile_protos(&["proto/handshaker.proto"], &["proto/"])?;
    Ok(())
}
