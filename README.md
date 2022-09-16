![Bookx_github_description](data/screenshots/Bookx_github_description.png)

# Bookx
An MVP in progress:
- [ ] An ebook reader with .epub support
    - [ ] Context menu for each book (delete, rename book, info)
    - [ ] On click switch the carousal to the book
- [ ] Ebook editor for .epub files

<div align="center">

![Main window](data/screenshots/screenshot1.png)

</div>

## Build & Run the project

Building the project

```bash
flatpak install org.gnome.Sdk//41 org.freedesktop.Sdk.Extension.rust-stable//21.08 org.gnome.Platform//41
flatpak-builder --user flatpak_app build-aux/com.adhadse.Bookx.Devel.json
```

To run the project

```bash
flatpak-builder --run flatpak_app build-aux/com.adhadse.Bookx.Devel.json bookx
```
> Please note that these commands are just for demonstration purposes. Normally this would be handled by your IDE, such as GNOME Builder or VS Code with the Flatpak extension.

