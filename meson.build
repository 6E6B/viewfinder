project('viewfinder', 'rust',
  version: '0.1.0',
  meson_version: '>= 1.3.0',
)

gnome = import('gnome')
cargo = find_program('cargo', required: true)
bash = find_program('bash', required: true)

prefix = get_option('prefix')
bindir = get_option('bindir')
datadir = get_option('datadir')

custom_target(
  'viewfinder',
  output: 'viewfinder',
  command: [
    bash, files('build-aux/cargo-build.sh'), cargo,
    meson.project_source_root(), meson.project_build_root() / 'cargo', '@OUTPUT@',
  ],
  build_by_default: true,
  build_always_stale: true,
  install: true,
  install_dir: bindir,
  console: true,
)

install_data('data/io.github._6E6B.viewfinder.desktop',
  install_dir: datadir / 'applications',
)
install_data('data/io.github._6E6B.viewfinder.metainfo.xml',
  install_dir: datadir / 'metainfo',
)
install_data('data/io.github._6E6B.viewfinder.svg',
  install_dir: datadir / 'icons' / 'hicolor' / 'scalable' / 'apps',
)

service_conf = configuration_data()
service_conf.set('bindir', prefix / bindir)
configure_file(
  input: 'data/io.github._6E6B.viewfinder.service.in',
  output: 'io.github._6E6B.viewfinder.service',
  configuration: service_conf,
  install: true,
  install_dir: datadir / 'dbus-1' / 'services',
)

desktop_file_validate = find_program('desktop-file-validate', required: false)
if desktop_file_validate.found()
  test(
    'Validate desktop file',
    desktop_file_validate,
    args: [meson.project_source_root() / 'data' / 'io.github._6E6B.viewfinder.desktop'],
  )
endif

appstreamcli = find_program('appstreamcli', required: false)
if appstreamcli.found()
  test(
    'Validate metainfo',
    appstreamcli,
    args: [
      'validate', '--no-net', '--explain',
      meson.project_source_root() / 'data' / 'io.github._6E6B.viewfinder.metainfo.xml',
    ],
  )
endif

gnome.post_install(
  gtk_update_icon_cache: true,
  update_desktop_database: true,
)
