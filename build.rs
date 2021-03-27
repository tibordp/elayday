fn main() {
    tonic_build::compile_protos("proto/elayday.proto").unwrap();
}
