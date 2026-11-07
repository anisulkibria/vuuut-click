fn main() {
    // SQLx embeds migrations at compile time. Watch the directory so newly added
    // migration files are picked up by ordinary incremental cargo builds.
    println!("cargo:rerun-if-changed=../../migrations/postgres");
    println!("cargo:rerun-if-changed=../../migrations/clickhouse");
}
