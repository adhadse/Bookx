# Translation glossary

One translation per English term, per language. Translators (people and agents) use these words every time; a
change here is a change of all strings that contain the term. The table is what the current files use, with the
decision where the files disagree with themselves (marked **fix**). Terms that no current string uses yet (collection,
catalogue, e-reader in fr/hi, …) are recommendations for when they appear. Source of truth for the English terms:
`docs/human_interface_backlog.md` §7 (Sync, Devices, capitalisation).

## Rules for every language

- `{}` placeholders, `…`, `_` access keys, `<b>`-style markup, file names, `bookx://`, key names in backticks and
  the product names **Bookx**, **KOReader**, **Calibre**, **Readwise**, **OPDS**, **EPUB** stay as they are.
- `…` (U+2026, no space before it unless the language's typographic rule demands it: see the language notes) at the
  end of a label that opens a dialog. Never three dots.
- English **header capitalisation** (buttons, menu items, tooltips, dialog headings) is an English rule. German
  capitalises nouns anyway; Spanish and French use sentence case for those labels (GNOME convention); Hindi has no
  case. Do not carry the English capitals over.
- One address form per language (below). Never mix.
- Keep the word order of placeholders: `{}` are filled left to right, so rephrase rather than swap.
- `translator-credits`: leave it empty (`msgstr ""`) or equal to the English id: the app shows no credit line
  (owner decision: translations by agents carry no credit).

## Address form

| de | es | fr | hi |
|---|---|---|---|
| **Sie** (formal), imperative with "Sie" or the infinitive in labels; never "du" | **tú** (informal, GNOME es convention); labels use the infinitive ("Guardar"), sentences "elige", "puedes" | **vous**; labels use the infinitive ("Enregistrer") | आप; polite imperative ("चुनें", "खोलें") |

## Terms

| English | de | es | fr | hi |
|---|---|---|---|---|
| Library | Bibliothek | Biblioteca | Bibliothèque | लाइब्रेरी |
| Virtual Library | Virtuelle Bibliothek | Biblioteca virtual | Bibliothèque virtuelle | वर्चुअल लाइब्रेरी |
| Shelf / Shelves | Regal / Regale | Estante / Estantes | Étagère / Étagères | शेल्फ़ |
| Smart shelf | Intelligentes Regal | Estante inteligente | Étagère intelligente | स्मार्ट शेल्फ़ |
| Collection | Sammlung | Colección | Collection | संग्रह |
| Series | Reihe | Serie | Série | शृंखला |
| Edition | Ausgabe | Edición | Édition | संस्करण |
| Catalogue (the library database) | Katalog | Catálogo | Catalogue | सूची |
| Book | Buch | Libro | Livre | पुस्तक |
| Author / Authors | Autor / Autoren | Autor / Autores | Auteur / Auteurs | लेखक |
| Publisher | Verlag | Editorial | Éditeur | प्रकाशक |
| Cover | Cover (everywhere; not "Titelbild") **fix** | Portada (everywhere; not "cubierta") **fix** | Couverture | कवर |
| Book Jacket (metadata page) | Schutzumschlag | Sobrecubierta | Jaquette | जैकेट |
| Table of Contents | Inhaltsverzeichnis | Tabla de contenidos | Table des matières | विषय-सूची |
| Metadata | Metadaten | Metadatos | Métadonnées | मेटाडेटा |
| Highlight | Markierung | Subrayado | Surlignage | हाइलाइट |
| Note | Notiz | Nota | Note | नोट |
| Annotation | Anmerkung | Anotación | Annotation | टिप्पणी |
| Bookmark | Lesezeichen | Marcador | Marque-page | बुकमार्क |
| Reading progress / state | Lesefortschritt / Lesestand | Progreso / estado de lectura | Progression / état de lecture | पढ़ने की प्रगति |
| Rating | Bewertung | Valoración | Évaluation **fix** (not "Note") | रेटिंग |
| Tag | Tag | Etiqueta | Étiquette | टैग |
| Backup (noun, the copy) | Sicherung | Copia de seguridad | Sauvegarde | बैकअप |
| Back up (verb) | sichern | hacer una copia de seguridad | sauvegarder | बैकअप लें |
| Restore | Wiederherstellen | Restaurar | Restaurer | पुनर्स्थापित करें |
| Checkpoint (editor undo point) | Prüfpunkt | Punto de control | Point de contrôle | चेकपॉइंट |
| Sync (menu, page: the decided word) | Sync (verb in sentences: synchronisieren) | Sincronización (verb: sincronizar) | Synchronisation (verb: synchroniser) | सिंक |
| Device | Gerät | Dispositivo | Appareil | डिवाइस |
| E-reader | E-Reader | Lector electrónico | Liseuse | ई-रीडर |
| Preferences | Einstellungen | Preferencias | Préférences | प्राथमिकताएँ |
| Settings | Einstellungen | Ajustes | Réglages | सेटिंग |
| Search (noun) / Find | Suche / Suchen | Búsqueda / Buscar | Recherche / Rechercher | खोज / ढूँढें |
| Saved search | Gespeicherte Suche | Búsqueda guardada | Recherche enregistrée | सहेजी गई खोज |
| Dictionary | Wörterbuch | Diccionario | Dictionnaire | शब्दकोश |
| Vocabulary | Vokabular | Vocabulario | Vocabulaire | शब्दावली |
| Read Aloud | Vorlesen | Leer en voz alta | Lecture à voix haute | ज़ोर से पढ़ें |
| Audiobook | Hörbuch | Audiolibro | Livre audio | ऑडियोबुक |
| Stylesheet | Stylesheet | Hoja de estilo (plural: hojas de estilo) **fix** | Feuille de style | स्टाइलशीट |
| Font | Schrift | Fuente | Police | फ़ॉन्ट |
| Trash | Papierkorb | Papelera | Corbeille | कचरा |
| Duplicates | Duplikate | Duplicados | Doublons | डुप्लिकेट |
| Convert | Konvertieren | Convertir | Convertir | बदलें |
| Plugin | Plugin | Complemento | Extension | प्लगइन |
| Patch (user patch, see wiki) | Patch | Parche | Correctif | पैच |
| Command Palette | Befehlspalette | Paleta de comandos | Palette de commandes | कमांड पैलेट |
| Snippet | Textbaustein | Fragmento | Extrait | स्निपेट |
| Offline mode | Offline-Modus | Modo sin conexión | Mode hors ligne | ऑफ़लाइन मोड |
| Save / Cancel / Open | Speichern / Abbrechen / Öffnen | Guardar / Cancelar / Abrir | Enregistrer / Annuler / Ouvrir | सहेजें / रद्द करें / खोलें |
| Undo / Redo | Rückgängig / Wiederholen | Deshacer / Rehacer | Annuler / Rétablir | पूर्ववत करें / फिर से करें |

## Language notes

- **de**: ellipsis attached (`Ändern…`), not `Ändern …` (the files mix both: **fix** to attached, as in GNOME de).
  Decimal comma, dates `03.11.2026`. "Sie" capitalised.
- **es**: `«»` for quotes, `p. ej.` for e.g.; "hoja de estilo" never "hoja de estilos".
- **fr**: typographic apostrophe `’` everywhere (the file mixes `'` and `’`: **fix** to `’`); narrow no-break
  space before `: ; ? !` (existing files use a normal space, accepted); `«\u00a0»` for quotes.
- **hi**: GNOME Hindi keeps well-known English loanwords in Devanagari (डिवाइस, लाइब्रेरी, बैकअप); use `{} मिलान`, not
  `{} मेल` ("मेल" reads as e-mail). Plural rule `n > 1` (0 and 1 singular).
