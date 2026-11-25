[Goodtype](../README.md) / Files and import

# Bring existing work into Goodtype

In the Library, choose **New → Import notebooks** and select your source. The optional [Goodnotes importer](https://github.com/goodtype-notes/Goodtype-Goodnotes-Importer) reads `.goodnotes` files and backup ZIPs. Handwriting and supported shapes remain editable. Other supported artwork appears as cropped vector material.

The [OneNote importer](https://github.com/goodtype-notes/Goodtype-OneNote-Importer) reads sections, `.onepkg` packages and web exports. It converts typed text and equations to Typst, keeps ink and pictures, and splits tall pages at gaps. Check the resulting notebook against the original, especially for complex layouts.

Goodtype downloads each importer when you request it. Conversion runs locally and preserves the source. Installed importers work offline. Updates are explicit and verified against signed release metadata.

You can also place an image or PDF material on an existing page, or import PDF pages as notebook pages. Cropping and annotation preserve the original asset.

## Back up a notebook

Copy the entire notebook directory. It contains the JSON manifest and pages, Typst source in `.typ` files, calculation source and saved outputs in `.ipynb` files, vector ink, and original assets. The bundled ink decoder can print a binary ink layer as JSON. [Ink format and migration](../formats/ink-v1.md)

Current builds read schemas 1 through 5. Ordinary notebooks use schema 3. Adding a calculation page requires schema 4. Pages with addon data or stored ports require schema 5. Older builds cannot open newer schemas. Saving an older page migrates its ink and preserves the original JSON files.

Stretchable symbols store an optional `stretch` field on the schema-5 Group.
It records the axis port indices, child IDs and roles, and the Group-local
length limits. An older schema-5 reader can open the saved shapes and ports.
Saving with that reader can remove the stretch controls while preserving the
last saved artwork and port positions.

## Work offline

Core editing, Typst compilation and PDF export run locally. Typst packages use the local cache first. You can disable package downloads and keep using cached packages. Python runtime downloads have a separate setting. Saved calculations remain readable without Python.

[Notebook guide](notebooks.md)
