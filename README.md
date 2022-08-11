![Bookx_github_description](https://user-images.githubusercontent.com/56764399/183672053-09a598fe-d6ef-4cf4-8f75-b898012e00fe.png)

# Bookx
An MVP in progress:
- [ ] An ebook reader with .epub support
- [ ] Ebook editor for .epub files

<div align="center">

![Main window](data/screenshots/screenshot1.png)

</div>

## Build & Run the project

Building the project

```bash
flatpak install org.gnome.Sdk//41 org.freedesktop.Sdk.Extension.rust-stable//21.08 org.gnome.Platform//41
flatpak-builder --user flatpak_app build-aux/com.anuragdhadse.Bookx.Devel.json
```

To run the project

```bash
flatpak-builder --run flatpak_app build-aux/com.anuragdhadse.Bookx.Devel.json bookx
```
> Please note that these commands are just for demonstration purposes. Normally this would be handled by your IDE, such as GNOME Builder or VS Code with the Flatpak extension.

