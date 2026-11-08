fn main() {
    kompitum_core::legacy::adopt_env();
    std::process::exit(kompitum_core::cli::main());
}
