project(
  'mq-mail',
  version: '0.1.0',
  license: 'GPL-3.0-or-later',
  meson_version: '>= 0.62.0',
)

# Dependencies for validation only (actual linking handled by Cargo)
dependency('gtk4', version: '>= 4.14')
dependency('libadwaita-1', version: '>= 1.4')
dependency('webkit2gtk-6.0')

gnome = import('gnome')
i18n = import('i18n')

app_id = 'com.mqmail.MqMail'
prefix = get_option('prefix')
bindir = prefix / get_option('bindir')
datadir = prefix / get_option('datadir')

# --- Install the Cargo-built binary ---
cargo_build = find_program('build-aux' / 'cargo-build.sh')

custom_target(
  'mq-mail-binary',
  output: 'mq-mail',
  command: [cargo_build, '@OUTPUT@'],
  install: true,
  install_dir: bindir,
  console: true,
)

# --- Desktop file ---
install_data(
  'data' / app_id + '.desktop',
  install_dir: datadir / 'applications',
)

# --- AppStream metainfo ---
install_data(
  'data' / app_id + '.metainfo.xml',
  install_dir: datadir / 'metainfo',
)

# --- Application icon ---
install_data(
  'data' / app_id + '.svg',
  install_dir: datadir / 'icons' / 'hicolor' / 'scalable' / 'apps',
)

# --- CSS stylesheet ---
install_data(
  'data' / 'resources' / 'style.css',
  install_dir: datadir / 'mq-mail',
)
