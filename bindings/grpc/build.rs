fn main() -> Result<(), Box<dyn std::error::Error>> {
  //tonic_build::compile_protos("proto/helloworld.proto")?;
  let proto_files = std::fs::read_dir("./proto")?
    .filter_map(|entry| entry.ok().map(|e| e.path()))
    .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("proto"));

  for proto in proto_files {
    tonic_build::compile_protos(proto)?;
  }

  Ok(())
}
