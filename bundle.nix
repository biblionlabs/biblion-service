{ pkgs, lib, format, target, pname, drv, ... }: let
  packageMeta = {
    description = ''Zerac is a true zero-trust security platform that protects any critical\
resource through end-to-end encryption for every interaction and low-latency\
direct connections between users or machines and their\
protected resources in one easy-to-implement'';
    version = "1.0.0";
    maintainer = "Zerac Team <support@zerac.com>";
    downloadUrl = "https://zerac.com";
    homepage = "https://zerac.com";
    deb = {
      depends = ["libc6"];
      recommends = [];
      section = "utils";
      priority = "optional";
    };
    rpm = {
      requires = ["glibc"];
      recommends = [];
      group = "Development/Tools";
    };
    archlinux = {
      requires = ["glibc"];
      optionals = [];
    };
    systemd = []; # or []
  };

  copyNonStandardLibs = drv: destDir: ''
    echo "Filtering libraries in ${drv}/lib..."

    standard_libs=(
      "libc.so" "libm.so" "libgcc_s.so" "libstdc++.so" "ld-linux"
      "libdl.so" "libpthread.so" "librt.so" "libutil.so"
      "libgomp.so" "libatomic.so" "libasan.so" "libtsan.so"
    )

    for lib in "$(cat ${pkgs.referencesByPopularity drv})"
    do
      if ! [[ -d "$lib/lib" ]]; then
        continue
      fi

      if ! [[ -d "${destDir}lib" ]]; then
        mkdir -p ${destDir}lib
      fi

      for std_lib in "$\{standard_libs[@]}"; do
      if [[ "$libname" == *"$std_lib"* ]]; then
          is_standard=1
          break
      fi
      done

      if [[ $is_standard -eq 0 ]]; then
        echo "Copying non-standard library: $lib"
        cp -r $lib/lib ${destDir}lib/$(basename "$lib")
      fi
      # ${pkgs.patchelf}/bin/patchelf --set-rpath '$ORIGIN/../lib' ${destDir}lib/$(basename "$lib")
    done
  '';

  handleSystemd = drv: meta: destDir:
    if meta ? systemd then
      ''
        mkdir -p ${destDir}
      '' + (
      if lib.isList meta.systemd then
        lib.concatMapStringsSep "\n" (service: ''
          cp ${service} ${destDir}$(basename ${service})
          chmod 644 ${destDir}$(basename ${service})
        '') meta.systemd
      else
        ''
          cp ${meta.systemd} ${destDir}${drv.pname}.service
          chmod 644 ${destDir}${drv.pname}.service
        '')
    else "";

  prepareOptZeracStructure = drv: destDir: ''
    mkdir -p ${destDir}bin
    mkdir -p ${destDir}lib

    for bin in ${drv}/bin/*; do
      install -Dm755 "$bin" "${destDir}bin/$(basename "$bin")"

      if ! [ "${target.os}" == "windows" ]; then
        ${pkgs.patchelf}/bin/patchelf \
            --set-interpreter "/usr/lib/ld-linux-${pkgs.lib.strings.replaceStrings ["_"] ["-"] target.arch}.so.2" \
            --set-rpath "\$ORIGIN/../lib:/usr/lib:/lib" \
            "${destDir}bin/$(basename "$bin")"
      fi
    done
  '';

  compress = ext: drv: let
    meta = packageMeta // (target.packageMeta or {});
    path = if target.os != "windows" then "package/opt/zerac/" else "";
    fromPath = if target.os != "windows" then "package/*" else "bin/* lib/*";
  in pkgs.stdenv.mkDerivation {
    dontUnpack = true;
    name = "${pname}-${ext}";
    buildInputs = with pkgs; [ ouch patchelf ];

    buildPhase = ''
      ${lib.optionalString (target.os != "windows") ''
        mkdir -p package/opt/zerac/{bin,lib}
      ''}

      ${prepareOptZeracStructure drv path}

      if [[ -d "${drv}/resources" ]]; then
        cp -r ${drv}/resources ${path}
      fi

      ${lib.optionalString (target.os == "linux") ''
        mkdir -p package/lib/systemd/system
        ${handleSystemd drv meta "package/lib/systemd/system/"}
      ''}

      ${copyNonStandardLibs drv path}
    '';

    installPhase = ''
      mkdir -p $out
      ouch compress ${fromPath} "$out/${pname}-${meta.version}.${ext}"
    '';
  };

  deb = drv: let
    meta = packageMeta // (target.packageMeta or {});
  in pkgs.stdenv.mkDerivation {
    dontUnpack = true;
    name = "${target.pname}-${meta.version}.deb";
    nativeBuildInputs = with pkgs; [ dpkg fakeroot patchelf ];

    buildCommand = ''
      mkdir -p debian-package/DEBIAN
      mkdir -p debian-package/opt/zerac
      mkdir -p debian-package/lib/systemd/system

      ${prepareOptZeracStructure drv "debian-package/opt/zerac/" }

      ${handleSystemd drv meta "debian-package/lib/systemd/system/"}

      cat > debian-package/DEBIAN/control <<EOF
      Package: ${target.pname}
      Version: ${meta.version}
      Architecture: ${if target.arch == "x86_64" then "amd64" else target.arch}
      Maintainer: ${meta.maintainer}
      Description: ${meta.description}
      Homepage: ${meta.homepage}
      Section: ${meta.deb.section or "utils"}
      Priority: ${meta.deb.priority or "optional"}
      ${lib.optionalString (meta.deb.depends != []) "Depends: ${lib.concatStringsSep ", " meta.deb.depends}\n"}
      ${lib.optionalString ((meta.deb.recommends or []) != []) "Recommends: ${lib.concatStringsSep ", " (meta.deb.recommends or [])}\n"}
      EOF

      cat > debian-package/DEBIAN/postinst <<EOF
      #!/bin/sh
      ${lib.optionalString (meta ? systemd) "systemctl daemon-reload"}
      EOF

      cat > debian-package/DEBIAN/postrm <<EOF
      #!/bin/sh
      ${lib.optionalString (meta ? systemd) "systemctl daemon-reload"}
      EOF

      mkdir -p $out

      mv debian-package $out/debian-package

      chmod +x $out/debian-package/DEBIAN/postinst
      chmod +x $out/debian-package/DEBIAN/postrm

      fakeroot dpkg-deb --build "$out/debian-package" "$out/${target.pname}-${meta.version}.deb"
    '';
  };

  archlinux = drv: let
    meta = packageMeta // (target.packageMeta or {});
  in pkgs.stdenv.mkDerivation {
    dontUnpack = true;
    name = "${pname}-archlinux";

    buildPhase = ''
      mkdir -p pkg/archlinux

      cp ${compress "tar.xz" drv}/${pname}-${meta.version}.tar.xz pkg/

      cat > pkg/archlinux/PKGBUILD <<EOF
# Maintainer: ${meta.maintainer}
pkgname=${target.pname}
pkgver=${meta.version}
pkgrel=1
pkgdesc="${meta.description}"
arch=('${if target.arch == "x86_64" then "x86_64" else target.arch}')
url="${meta.homepage}"
license=('Proprietary')
depends=(${lib.concatStringsSep " " meta.archlinux.requires})
${lib.optionalString (meta ? systemd) "backup=(${lib.concatMapStrings (s: "'/usr/lib/systemd/system/$(basename ${s})' ") meta.systemd})"}

package() {
  install -dm755 "\$pkgdir/opt/zerac"
  cp -r "\$srcdir/../opt/zerac/bin" "\$pkgdir/opt/zerac/"
  cp -r "\$srcdir/../opt/zerac/lib" "\$pkgdir/opt/zerac/"

  [ -d "\$srcdir/../opt/zerac/resources" ] && \\
    cp -r "\$srcdir/../opt/zerac/resources" "\$pkgdir/opt/zerac/"

  ${lib.optionalString (meta ? systemd) ''
    install -dm755 "\$pkgdir/usr/lib/systemd/system"
    ${lib.concatMapStrings (service: ''
      install -Dm644 "\$srcdir/../usr/lib/systemd/system/$(basename ${service})" \\
      "\$pkgdir/usr/lib/systemd/system/$(basename ${service})"
    '') (if lib.isList meta.systemd then meta.systemd else [meta.systemd])}
  ''}

  install -dm755 "\$pkgdir/usr/bin"
  for bin in \$(ls "\$pkgdir/opt/zerac/bin"); do
    ln -s "/opt/zerac/bin/\$bin" "\$pkgdir/usr/bin/\$bin"
  done
}
EOF

      cat > pkg/archlinux/.SRCINFO <<EOF
pkgbase = ${target.pname}
pkgdesc = ${meta.description}
pkgver = ${meta.version}
pkgrel = 1
url = ${meta.homepage}
arch = ${if target.arch == "x86_64" then "x86_64" else target.arch}
license = Proprietary
${lib.concatMapStrings (d: "depends = ${d}\n") meta.archlinux.requires}
pkgname = ${target.pname}
EOF
    '';

    installPhase = ''
      mkdir -p $out

      cp pkg/${pname}-${meta.version}.tar.xz $out/${pname}-${meta.version}.tar.xz
      cp -r pkg/archlinux/{PKGBUILD,.SRCINFO} "$out/"
    '';
  };

  rpm = drv: let
    meta = packageMeta // (target.packageMeta or {});
  in pkgs.stdenv.mkDerivation {
    dontUnpack = true;
    name = "${target.pname}-${meta.version}-${target.arch}.rpm";
    nativeBuildInputs = with pkgs; [ pkgs.rpm patchelf ];

    buildCommand = ''
      export HOME=$(mktemp -d)
      export buildbase=$HOME/rpmbuild
      export buildroot=$buildbase/BUILDROOT/${target.pname}-${meta.version}-1.${target.arch}
      export RPM_DB_PATH=$HOME/.rpmdb

      mkdir -p $RPM_DB_PATH
      mkdir -p $buildbase/{BUILD,RPMS,SOURCES,SPECS,SRPMS}
      mkdir -p $buildroot/opt/zerac
      mkdir -p $buildroot/lib/systemd/system

      rpm --initdb --dbpath "$RPM_DB_PATH"

      ${prepareOptZeracStructure drv true}

      mv "$out/opt/zerac" "$buildroot/opt/zerac"

      ${handleSystemd drv true meta "/rpmbuild/BUILDROOT/${target.pname}-${meta.version}-1.${target.arch}/lib/systemd/system/"}

      cat > $buildbase/SPECS/zerac.spec <<EOF
Name: Zerac
Version: ${meta.version}
Release: 1%{?dist}
Summary: ${meta.description}
License: Proprietary
URL: ${meta.homepage}
Group: ${meta.rpm.group or "Applications/System"}
BuildArch: ${target.arch}
${lib.optionalString (meta.rpm.requires != []) "Requires: ${lib.concatStringsSep ", " meta.rpm.requires}\n"}\
${lib.optionalString ((meta.rpm.recommends or []) != []) "Recommends: ${lib.concatStringsSep ", " (meta.rpm.recommends or [])}\n"}\

%description
${meta.description}

%files
%attr(0755, root, root) /opt/zerac/*
${lib.optionalString (meta ? systemd) "%attr(0644, root, root) /lib/systemd/system/*"}

%post
${lib.optionalString (meta ? systemd && meta.systemd != []) "systemctl daemon-reload >/dev/null 2>&1 || :"}

%postun
${lib.optionalString (meta ? systemd && meta.systemd != []) "systemctl daemon-reload >/dev/null 2>&1 || :"}

%clean
rm -rf \$RPM_BUILD_ROOT
EOF

      rpmbuild \
        --define "_topdir $buildbase" \
        --dbpath "$RPM_DB_PATH" \
        --target ${target.arch} \
        -bb $buildbase/SPECS/zerac.spec

      mkdir -p $out/rpm
      cp $buildbase/RPMS/${target.arch}/${target.pname}-${meta.version}-1.${target.arch}.rpm $out/rpm/
    '';
  };
in
  if format == "deb" then
    deb drv
  else if format == "rpm" then
    rpm drv
  else if format == "archlinux" then
    archlinux drv
  else
    compress format drv
