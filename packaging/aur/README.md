# Publicación de sane-rec-bin en AUR (Arch User Repository)

El Arch User Repository (AUR) es un repositorio comunitario y abierto. Como autor legítimo del proyecto, publicar tu paquete es un proceso directo que toma 2 minutos y no requiere aprobaciones ni trámites burocráticos.

---

### Paso 1: Crear una cuenta en AUR (si aún no tienes una)
1. Ve a [https://aur.archlinux.org/register](https://aur.archlinux.org/register) y regístrate.
2. En tu perfil de AUR (`My Account`), pega tu clave pública SSH (`~/.ssh/id_ed25519.pub` o `~/.ssh/id_rsa.pub`).

---

### Paso 2: Clonar el repositorio del paquete en AUR
En tu terminal:
```bash
git clone ssh://aur@aur.archlinux.org/sane-rec-bin.git /tmp/aur-sane-rec-bin
```

---

### Paso 3: Copiar los archivos ya generados
Copia los archivos `PKGBUILD` y `.SRCINFO` de esta carpeta al repositorio clonado:
```bash
cp PKGBUILD .SRCINFO /tmp/aur-sane-rec-bin/
```

---

### Paso 4: Subir el paquete
```bash
cd /tmp/aur-sane-rec-bin
git add PKGBUILD .SRCINFO
git commit -m "feat: initial release v0.1.0"
git push origin master
```

**¡Listo!** En ese instante exacto, cualquier usuario de CachyOS, Arch Linux, Manjaro o EndeavourOS podrá instalarlo ejecutando:
```bash
yay -S sane-rec-bin
```
