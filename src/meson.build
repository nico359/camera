pkgdatadir = get_option('prefix') / get_option('datadir') / meson.project_name()
gnome = import('gnome')

gnome.compile_resources('camera',
  'camera.gresource.xml',
  gresource_bundle: true,
  install: true,
  install_dir: pkgdatadir,
)

conf = configuration_data()
conf.set_quoted('VERSION', meson.project_version())
conf.set_quoted('GETTEXT_PACKAGE', 'camera')
conf.set_quoted('LOCALEDIR', get_option('prefix') / get_option('localedir'))
conf.set_quoted('PKGDATADIR', pkgdatadir)

configure_file(
    input: 'config.rs.in',
    output: 'config.rs',
    configuration: conf
)

# Copy the config.rs output to the source directory.
run_command(
  'cp',
  meson.project_build_root() / 'src' / 'config.rs',
  meson.project_source_root() / 'src' / 'config.rs',
  check: true
)

cargo_bin = find_program('cargo')
cargo_home = meson.project_build_root() / 'cargo-home'
cargo_target_dir = meson.project_build_root() / 'src'

cargo_args = []
if get_option('buildtype') == 'release'
  cargo_args += ['--release']
  rust_target = 'release'
else
  rust_target = 'debug'
endif

if get_option('camera2')
  cargo_args += ['--features', 'camera2']
endif

cargo_build = custom_target(
  'cargo-build',
  build_by_default: true,
  build_always_stale: true,
  output: meson.project_name(),
  console: true,
  install: true,
  install_dir: get_option('bindir'),
  command: [
    find_program('sh'),
    meson.project_source_root() / 'build-aux' / 'cargo-build.sh',
    cargo_bin.full_path(),
    cargo_home,
    meson.project_source_root() / 'Cargo.toml',
    cargo_target_dir,
    cargo_target_dir / rust_target / meson.project_name(),
    '@OUTPUT@',
  ] + cargo_args,
)
