# rmsctl

CLI-Client fuer die NeoDEM Roboter-Flottenmanagement-Plattform.

![rmsctl demo](tapes/demo.gif)

## Installation

```bash
cargo build --release
cp target/release/rmsctl ~/.local/bin/
```

## Konfiguration

Ohne Konfiguration verbindet sich `rmsctl` mit `http://localhost:3001/api` (kein Token noetig im Dev-Modus).

```bash
# Per Flag
rmsctl --server http://host:3001/api --token <jwt> status

# Per Umgebungsvariable
export RMS_SERVER_URL=http://host:3001/api
export RMS_TOKEN=<jwt>

# Per Konfigurationsdatei
cat ~/.config/rmsctl/config.json
```

```json
{
  "server_url": "http://localhost:3001/api",
  "token": "optional-jwt"
}
```

Reihenfolge: CLI-Flag > Umgebungsvariable > Konfigurationsdatei > Default.

`~/.config/rmsctl/config.json` wird auf allen Plattformen zuerst gelesen. Zusaetzlich
wird das plattformspezifische Config-Verzeichnis als Fallback geprueft (macOS:
`~/Library/Application Support/rmsctl/config.json`, Windows: `%APPDATA%\rmsctl\config.json`).

## Globale Flags

```
--json           Server-Payload unveraendert als JSON ausgeben
--yes            Zustandsaendernde Befehle wirklich ausfuehren
--page <N>       Seite einer Liste
--limit <N>      Eintraege pro Seite
--server <url>   Server-URL
--token <jwt>    Auth-Token
```

### Der `--yes`-Schalter

Jeder Befehl, der Flotten- oder Deployment-Zustand aendert, beschreibt ohne `--yes`
nur, was er tun wuerde, und bricht ab:

```bash
$ rmsctl safety estop sim-robot-001 --reason "Kollisionsgefahr"

  [!] This will ENGAGE EMERGENCY STOP on sim-robot-001 — reason: Kollisionsgefahr
  [-->] Re-run with --yes to execute.
```

Lesende Befehle sind nie gesperrt.

## Befehle

### Flotte & Sicherheit

```bash
rmsctl status                                    # Flotten-Dashboard (inkl. E-Stop-Status)
rmsctl robots list                               # Alle Roboter auflisten
rmsctl robots show <id>                          # Roboter-Details
rmsctl robots telemetry <id> [--history]         # Telemetrie (CPU, Joints, Sensoren)
rmsctl robots agent-mode <id>                    # Agent-Mode & E-Stop eines Roboters
rmsctl robots cameras <id>                       # Kameras
rmsctl robots peers <id>                         # A2A-Peers
rmsctl robots register <url>                     # Roboter registrieren
rmsctl robots remove <id>                        # Roboter entfernen
rmsctl robots command <id> <type> [--payload {}] # Befehl senden

rmsctl safety status [--robot <id>]              # E-Stop- und Sicherheitszustand
rmsctl safety estop <id> --reason <text>         # Nothalt ausloesen
rmsctl safety estop --fleet --reason <text>      # Flotten-Nothalt
rmsctl safety reset <id> | --fleet               # Nothalt zuruecksetzen
rmsctl safety zone-estop <zone-id> --reason <t>  # Nothalt fuer eine Zone
rmsctl safety events                             # Sicherheits-Ereignisse
```

### Alerts & Incidents

```bash
rmsctl alerts list [--active] [--severity X]     # Alerts
rmsctl alerts show <id>                          # Einzelner Alert
rmsctl alerts ack <id>                           # Alert bestaetigen
rmsctl alerts history                            # Alert-Historie
rmsctl alerts delete <id>                        # Alert loeschen
rmsctl alerts clear --acknowledged | --all       # Alerts massenweise loeschen

rmsctl incidents list [--status X] [--severity X] [--robot-id X]
rmsctl incidents show <id>                       # Incident-Details
rmsctl incidents dashboard                       # Incident-Rollup
rmsctl incidents open | overdue | risk-matrix
rmsctl incidents evidence <id>                   # Angehaengte Nachweise
```

### Aufsicht & Freigaben

```bash
rmsctl oversight dashboard | fleet               # Aufsichts-Uebersicht
rmsctl oversight anomalies [--active] [--unacked]
rmsctl oversight ack-anomaly <id>
rmsctl oversight resolve-anomaly <id> [--notes X]
rmsctl oversight manual-sessions                 # Manuelle Kontrollsitzungen
rmsctl oversight manual-mode <robot-id> [--reason X] [--off]
rmsctl oversight verifications [--due]
rmsctl oversight capabilities <robot-id>
rmsctl oversight logs

rmsctl approvals list | overdue | nearing        # Freigabe-Anfragen
rmsctl approvals pending [--role <r>]            # Was auf dich wartet
rmsctl approvals show <id>
rmsctl approvals decide <id> <step-id> --decision approve|reject|defer|request_info [--notes X]
rmsctl approvals cancel <id> | escalate <id>
rmsctl approvals metrics | sla-report
```

### Teleoperation & Patrouille

```bash
rmsctl teleop list [--robot-id X] [--status X] [--type X]
rmsctl teleop show <id> | episodes <id>
rmsctl teleop pause <id> | resume <id> | end <id>

rmsctl patrol routes | route <id>                # Patrouillen-Routen
rmsctl patrol runs [--route <id>] | run <run-id> # Laeufe
rmsctl patrol findings | finding <id>            # Befunde
rmsctl patrol places                             # Bekannte Orte
rmsctl patrol start <route-id> [--robot-id X] [--mode baseline|patrol]
rmsctl patrol abort <route-id> [--reason X]
rmsctl patrol ack-finding <id> | normal-finding <id> | escalate-finding <id>
rmsctl patrol promote-run <run-id>               # Lauf als Baseline uebernehmen
```

### Daten & Training

```bash
rmsctl datasets list                             # Datensaetze auflisten
rmsctl datasets show <id>                        # Datensatz-Details
rmsctl datasets episodes <id>                    # Episoden eines Datensatzes
rmsctl datasets quality <id> | stats <id>        # Qualitaet & Statistik
rmsctl datasets validate <id> [--advanced]       # Validierung
rmsctl datasets push <id> --hf-token X --repo-id org/name
rmsctl datasets push-status <id>
rmsctl datasets upload <dir> [--name X]          # Datensatz-Verzeichnis direkt hochladen, mit allen Dateien (ohne Hugging Face)
rmsctl datasets verify <id> <dir>                # Pruefen, dass alles aus <dir> auf der Instanz liegt (vor dem Loeschen!)
rmsctl datasets upload-status <id>               # Was der Server von einem Upload haelt
rmsctl datasets upload-cancel <id>               # Upload abbrechen, Gespeichertes loeschen
rmsctl datasets download <id> [--out <dir>]      # Datensatz als LeRobot-Verzeichnis zurueckholen
rmsctl datasets flagged <id>                     # Markierte Episoden
rmsctl datasets flag <id> <index> [--reason X]

rmsctl training list                             # Trainingsjobs auflisten
rmsctl training show <id>                        # Job-Details & Fortschritt
rmsctl training active | queue | workers         # Laufend, Warteschlange, Worker
rmsctl training start --config job.json          # Job starten
rmsctl training cancel <id> | retry <id>
rmsctl training export <id> | estimate <id>
```

#### Datensatz direkt hochladen

`rmsctl datasets upload` schickt ein LeRobot-Verzeichnis (v2.x oder v3.0) vom eigenen
Rechner direkt in eine RMS-Instanz — ohne Umweg ueber Hugging Face. Der Datensatz landet
im selben Zustand wie nach einem Hub-Import: Datensatzseite, Episoden-Browser, Video und
Training funktionieren unveraendert.

**Hochgeladen wird jede regulaere Datei des Verzeichnisses** — der LeRobot-Baum, die
`README.md` und alles, was sonst darin liegt —, damit `datasets download` das Verzeichnis
spaeter Byte fuer Byte zurueckbringt.

```bash
# 1. Ansehen, was passieren wuerde (sendet nichts) — listet auch jede Datei, die NICHT mitgeht
rmsctl --server https://neodem.example/api datasets upload /data/g1-dex3-bottle-sorting-sim

# 2. Hochladen, auf die Validierung warten, Ergebnis ausgeben
rmsctl --server https://neodem.example/api datasets upload /data/g1-dex3-bottle-sorting-sim \
  --name "G1 Dex3 bottle sorting" --yes

# 3. Bevor das lokale Verzeichnis geloescht wird: pruefen, dass alles angekommen ist
rmsctl --server https://neodem.example/api datasets verify <dataset-id> /data/g1-dex3-bottle-sorting-sim \
  && rm -rf /data/g1-dex3-bottle-sorting-sim
```

```
  ◈ Upload: G1 Dex3 bottle sorting
    │ Source         /data/g1-dex3-bottle-sorting-sim
    │ Format         LeRobot v3.0 · unitree_g1_dex3 · 30 fps · 80 episodes
    │ Files          9 (446.1 MB) — everything in the directory, 1 beside the LeRobot tree
    │ Left out       2 entries — NOT uploaded:
      - .git/  (version-control data)
      - raw/  (--exclude raw)
  [OK] Upload started: dataset 3fb4173c-… (rustfs store, 32 MiB parts, 21 part(s))
    │ Uploading      ████████░░░░░░░░░░░░  41.2%  183.8 MB / 446.1 MB · 6/9 files · 12.4 MB/s · ETA 21s
  [OK] Sent 446.1 MB in 36s (12.4 MB/s)
  [--] The server is assembling the files and verifying checksums …
  [OK] Dataset ready
    │ ID             3fb4173c-81e8-4311-aa13-1b1970f886fc
    │ Status         ● ready
    │ Episodes       80
    │ Frames         55 695
    │ Validation
    │ Result         valid
```

`<dir>` ist das Verzeichnis, das `meta/` und `data/` enthaelt. Ohne `videos/` entsteht
ein Datensatz im Modus `metadata`.

**Was nicht mitgeht** — und nur das:

| Ausgelassen | Warum |
| --- | --- |
| `.git`, `.DS_Store`, `__pycache__` (in jeder Tiefe), `.cache/` (nur ganz oben) | kein Datensatz-Inhalt: Versionsverwaltung, Finder, Python-Bytecode, der Download-Cache von `huggingface-cli` |
| `*.rmsctl-part` | angefangener `rmsctl`-Download; liesse sich nie zurueckholen |
| alles, was ein `--exclude <glob>` trifft | vom Aufrufer gewollt |
| leere Verzeichnisse, Sockets, Pipes | es werden Dateien gespeichert, keine Verzeichnisse |

Nichts davon faellt still weg: Der Lauf ohne `--yes` listet jeden ausgelassenen Eintrag
mit Grund (`Left out`), der Lauf mit `--yes` die ersten zwoelf und die Anzahl. Ein
Verzeichnis-Eintrag (`raw/`) steht fuer alles darunter.

**Symlinks** wird gefolgt (ein Hub-Cache-Snapshot besteht aus Links nach `blobs/`): Eine
verlinkte Datei oder ein verlinktes Verzeichnis wird mit dem Inhalt des Ziels
hochgeladen und kommt beim Download als normale Datei zurueck. Ein Link, dessen Ziel
fehlt, bricht den Befehl ab, bevor etwas gesendet wird — reparieren, entfernen oder per
`--exclude` auslassen. Ebenso ein Dateiname, den der Server ablehnen wuerde (Backslash,
Steuerzeichen, kein gueltiges UTF-8, Pfad ueber 1024 Bytes).

**Aeltere Server** (vor RMS TASK-500) speichern nur den LeRobot-Baum (`meta/**`,
`data/**/*.parquet`, `videos/**/*.mp4`, `images/**`) und lassen alles andere fallen. Das
erkennt `rmsctl` an der Antwort auf die Anmeldung, **bricht ab, bevor ein Byte gesendet
ist**, nimmt den geoeffneten Upload zurueck und nennt die Dateien, die verloren gingen
("this server stores only the LeRobot tree …"). Gegen einen solchen Server laedt
`--lerobot-only` wie frueher nur den Baum hoch — die uebrigen Dateien liegen dann
**nicht** auf der Instanz und duerfen lokal nicht geloescht werden.

| Flag | Bedeutung |
| --- | --- |
| `--exclude <glob>` | Dateien auslassen (wiederholbar), Syntax wie bei `models upload`: `*.log` und `raw` treffen in jeder Tiefe, `data/*/notes.txt` ist am Verzeichnis verankert, `**` ueberspringt Ebenen; ein Treffer auf ein Verzeichnis laesst alles darunter aus |
| `--lerobot-only` | Nur den LeRobot-Baum hochladen (fuer Server, die nichts anderes speichern) |
| `--no-default-excludes` | Auch `.git`, `.DS_Store`, `__pycache__` und `.cache/` hochladen |
| `--name`, `--description` | Name (Default: Verzeichnisname) und Beschreibung |
| `--robot-type <id>` | Robotertyp festlegen; sonst wird er aus `meta/info.json` zugeordnet oder angelegt |
| `--chunk-size <groesse>` | Teilgroesse, z. B. `8M` (min. 5M; Default und Maximum setzt der Server: 32M) |
| `--parallel <n>` | Gleichzeitig gesendete Teile (Default 3; der Server nimmt insgesamt 4 an) |
| `--retries <n>` | Versuche pro Teil nach voruebergehenden Fehlern, mit Backoff (Default 8) |
| `--resume <id>` | Einen bestimmten Upload fortsetzen (z. B. von einem anderen Rechner) |
| `--restart` | Den gemerkten Upload verwerfen und neu beginnen |
| `--no-checksum` | Kein SHA-256 ueber ganze Dateien (Teile werden weiterhin einzeln geprueft) |
| `--no-wait` | Nach dem letzten Teil zurueckkehren, nicht auf die Validierung warten |
| `--wait-timeout`, `--part-timeout` | Sekunden fuer das Urteil des Servers (1800) bzw. fuer einen Teil (300) |

- **Fortsetzen:** Bricht der Upload ab (Ctrl-C, Netz weg, Rechner aus), denselben Befehl
  noch einmal ausfuehren. `rmsctl` fragt den Server, welche Teile er schon hat, und sendet
  nur den Rest. Lokal wird nur gemerkt, welche Upload-ID zu welchem Verzeichnis gehoert
  (`~/.local/state/rmsctl/uploads/`, ueberschreibbar mit `RMSCTL_STATE_DIR`) — nie im
  Datensatz-Verzeichnis selbst.
- **Abbrechen:** `rmsctl datasets upload-cancel <id> --yes` entfernt alle gespeicherten
  Teile und den Datensatz-Eintrag. Das gilt auch fuer einen hochgeladenen Datensatz, den
  die Validierung abgelehnt hat. Ein Upload ohne Aktivitaet verfaellt serverseitig nach
  6 Stunden von selbst.
- **Speicherbedarf:** Dateien werden von der Platte gestreamt, nie ganz geladen. Der
  Speicherverbrauch haengt weder von Datei- noch von Teilgroesse noch von `--parallel` ab
  (gemessen: ca. 30 MB fuer 446 MB mit zwei 210-MB-Videos).
- **Pruefsummen:** Jeder Teil traegt seinen SHA-256 (`X-Chunk-Sha256`); zusaetzlich wird
  pro Datei ein SHA-256 angemeldet, den der Server nach dem Zusammensetzen gegenliest.

#### Vor dem Loeschen pruefen: `datasets verify`

`rmsctl datasets verify <id> <dir>` beantwortet die Frage "liegt alles aus diesem
Verzeichnis auf der Instanz?" — der Schritt zwischen `datasets upload` und dem Loeschen
der lokalen Kopie. Verglichen wird das Verzeichnis mit der Dateiliste der Instanz:
gleiche relative Pfade, gleiche Groessen, und der SHA-256 ueberall dort, wo die Instanz
einen hat (bei jedem Datensatz, der mit `rmsctl datasets upload` ohne `--no-checksum`
hochgeladen wurde). Geholt wird **nur die Dateiliste**, keine Datei; die lokalen Dateien
werden einmal gelesen und gehasht.

```
  ◈ Verify: g1-dex3-bottle-sorting-sim
    │ Dataset        ca244af7-4182-422a-bd33-d55dfa7478b4
    │ Status         ready
    │ Local          /data/g1-dex3-bottle-sorting-sim — 9 file(s), 446.1 MB
    │ Instance       9 file(s), 446.1 MB · rustfs store
  [--] Hashing 9 local file(s), 446.1 MB …
    │ Compared       path and size of every file · SHA-256 of 9
  [OK] Every one of the 9 file(s), 446.1 MB, is on the instance with the same path and size, and the same SHA-256 (9 files)
```

Jede Abweichung wird genannt, und der Befehl endet mit Exit 1:

```
  [!] 4 difference(s) between /data/g1-dex3-bottle-sorting-sim and the instance:
      - only on the instance     README.md  (11.8 kB — not in the directory)
      - missing on the instance  calib/new.yaml  (4 B locally)
      - content differs          data/chunk-000/file-000.parquet  (same size, SHA-256 edb088808a05… locally, 55577747fa46… on the instance)
      - size differs             meta/stats.json  (36656 bytes locally, 36654 on the instance)
  [ERR] /data/g1-dex3-bottle-sorting-sim does not match dataset 9bb8da21-…: 4 difference(s) — do not delete the local copy
```

- **Gleiche Regeln wie beim Upload:** Das Verzeichnis wird so gelesen, wie `datasets
  upload` es liest. Wer mit `--exclude raw` oder `--lerobot-only` hochgeladen hat, gibt
  dieselben Flags hier an; sonst zaehlen die ausgelassenen Dateien als "missing on the
  instance". Was die Regeln auslassen (`.git`, ein `--exclude`), wird unter `Left out`
  aufgelistet — es liegt nicht auf der Instanz und verschwindet mit dem Verzeichnis.
- **Auch die andere Richtung zaehlt:** Eine Datei, die nur auf der Instanz liegt, ist
  eine Abweichung. Nach einem erfolgreichen `verify` ist `download` + `diff -r` leer
  (bis auf die unter `Left out` genannten Eintraege).
- **Ohne SHA-256 auf der Instanz** (Hub-Import, Upload mit `--no-checksum`) entscheidet
  die Groesse; der Befehl sagt, wie viele Dateien nur so verglichen wurden.
  `--require-checksum` macht daraus einen Fehler, `--no-checksum` vergleicht absichtlich
  nur Groessen (schnell, liest keine Datei).
- **Datensatz von einem aelteren Server oder aelteren `rmsctl`:** dort fehlt z. B. die
  `README.md` auf der Instanz — `verify` meldet sie als "missing on the instance".
- **`--json`:** ein Objekt mit `match`, `localFiles`, `remoteFiles`, `sha256Compared`,
  `sizeOnly`, `differences[] { path, kind, detail }` und `leftOut[] { path, reason }`;
  `kind` ist `missingOnInstance`, `onlyOnInstance`, `size`, `sha256` oder `noChecksum`.
  Der Exit-Code ist derselbe.
- Der Befehl aendert nichts und braucht kein `--yes`. Server: braucht die Datei-API
  (RMS TASK-496).

#### Datensatz herunterladen

`rmsctl datasets download` holt einen Datensatz aus einer RMS-Instanz zurueck — als das
Verzeichnis, als das er gespeichert ist: `meta/`, `data/`, `videos/`, `README.md` … mit
denselben relativen Pfaden und denselben Bytes, direkt aus dem Speicher der Instanz, ohne
Hugging Face. Das ist der Rueckweg fuer Datensaetze, die mit `datasets upload` ausgelagert
wurden, um lokal Platz zu schaffen; es funktioniert genauso fuer Hub-Importe.

```bash
rmsctl --server https://neodem.example/api datasets download 6d5a0077-6b1f-4920-84a8-1c691903f8e0
rmsctl --server https://neodem.example/api datasets download 6d5a0077-… --out /data/g1-dex3-bottle-sorting-sim
```

```
  ◈ Download: g1-dex3-bottle-sorting-sim
    │ Dataset        6d5a0077-6b1f-4920-84a8-1c691903f8e0
    │ Status         ready
    │ Into           /data/g1-dex3-bottle-sorting-sim
    │ Files          8 · 446.1 MB · from the rustfs store
    │ Verified by    size and SHA-256
    │ Downloading    ████████░░░░░░░░░░░░  41.2%  183.8 MB / 446.1 MB · 6/8 files · 48.1 MB/s · ETA 5s
  [OK] Downloaded 8 file(s), 446.1 MB into /data/g1-dex3-bottle-sorting-sim
    │ Fetched        446.1 MB in 9s
    │ Verified       size of every file · SHA-256 of 8
```

Ohne `--out` entsteht `./<Datensatzname>`. Der Befehl aendert nichts auf dem Server und
braucht deshalb kein `--yes`.

| Flag | Bedeutung |
| --- | --- |
| `--out <dir>`, `-o` | Zielverzeichnis (Default: `./<Datensatzname>`) |
| `--parallel <n>` | Gleichzeitig geholte Dateien (Default 4, max. 16) |
| `--retries <n>` | Versuche pro Datei nach voruebergehenden Fehlern, mit Backoff (Default 8) |
| `--resume` | In einem nicht leeren Verzeichnis weitermachen: vollstaendige Dateien behalten, angefangene fortsetzen |
| `--force` | In ein nicht leeres Verzeichnis schreiben und jede Datei neu holen |
| `--no-checksum` | Nur Groessen pruefen, auch wenn der Server SHA-256-Werte liefert |
| `--stall-timeout <s>` | Sekunden ohne ein Byte, nach denen eine Anfrage neu gestellt wird (Default 60) |

- **Nicht leeres Verzeichnis:** wird ohne `--resume` oder `--force` abgelehnt (Exit 1),
  bevor irgendetwas geholt wird.
- **Fortsetzen:** Eine Datei heisst `<name>.rmsctl-part`, solange sie unvollstaendig ist,
  und bekommt ihren richtigen Namen erst, wenn Groesse (und SHA-256) stimmen. Nach einem
  Abbruch (Ctrl-C, Netz weg, `kill -9`) denselben Befehl mit `--resume` ausfuehren:
  Dateien mit passender Groesse werden uebersprungen, `.rmsctl-part`-Dateien per
  `Range`-Anfrage ab ihrer Laenge fortgesetzt. Reisst eine Verbindung mitten in einer
  Datei ab, macht `rmsctl` im selben Lauf genauso weiter. Der einzige Zustand ist das
  Verzeichnis selbst.
- **Pruefung:** Jede Datei wird gegen die Groesse aus der Dateiliste des Servers geprueft.
  Einen SHA-256 liefert der Server fuer Datensaetze, die mit `rmsctl datasets upload`
  (ohne `--no-checksum`) hochgeladen wurden; dann wird die ganze Datei nach dem Holen
  gegengelesen und bei Abweichung neu geholt. Hub-Importe haben keinen — dort gilt die
  Groesse.
- **Fehler:** Kann eine Datei nicht reproduziert werden (Datei auf dem Server
  verschwunden oder geaendert, Pruefsumme dauerhaft falsch, Versuche aufgebraucht), endet
  der Befehl mit Exit 1, nennt die Datei und laesst das bisher Geholte fuer `--resume`
  liegen. Pfade aus der Dateiliste, die das Zielverzeichnis verlassen wuerden, werden
  abgelehnt, bevor etwas geschrieben wird.
- **Rate-Limit:** Liste und Dateien zaehlen auf dem Server zum Budget des
  Datensatz-Browsers (3000 Anfragen pro Minute). `429` und `503` werden mit `Retry-After`
  abgewartet.
- **`--json`:** keine Fortschrittsanzeige; am Ende ein Objekt mit `datasetId`, `out`,
  `files`, `bytes`, `skippedFiles`, `fetchedBytes`, `verifiedSha256`.
- **Server:** braucht die Datei-API aus RaaSaaR-org/robot-management-system (TASK-496,
  `docs/dataset-download.md`). Ein aelterer Server wird mit "this server has no dataset
  download API" gemeldet.
- **Hinter einem Proxy:** Jede Anfrage traegt hoechstens einen Teil und wartet nie auf
  lange Serverarbeit. Auf einer langsamen Leitung `--chunk-size 8M` waehlen: ein Teil muss
  innerhalb des Proxy-Timeouts (ca. 100 s) ankommen.
- **Exit-Code:** 0 nur, wenn der Datensatz `ready` ist. Bei `failed` (Validierung oder
  Pruefsumme) werden die Befunde ausgegeben und der Befehl endet mit 1; Ctrl-C endet mit 130.
- **`--json`:** keine Fortschrittsanzeige; am Ende steht der unveraenderte
  `GET /datasets/<id>`-Payload auf stdout.

### Modelle & Deployment

```bash
rmsctl models list                               # Modell-Versionen (mit Basis, Quelle, Groesse)
rmsctl models show <id>                          # Version + Evaluierungs-Rollup
rmsctl models lineage <id>                       # Herkunft einer Version
rmsctl models compare <idA> <idB> [--metrics loss,acc]  # Metriken vergleichen
rmsctl models upload <dir> --version X [--name N]  # Checkpoint-Verzeichnis direkt hochladen und registrieren
rmsctl models upload-status [<id>]               # Ein Upload im Detail; ohne ID: alle offenen Uploads
rmsctl models upload-cancel <id>                 # Upload abbrechen, Gespeichertes loeschen
rmsctl models files <id>                         # Dateien, die die Instanz zu einem Modell speichert
rmsctl models download <id> [--out dir] [--resume]  # Modell herunterladen (prueft Groesse + SHA-256)
rmsctl models delete <id>                        # Archivieren; bei hochgeladenen Modellen auch die Dateien loeschen

rmsctl deployments list [--active]               # Deployments auflisten
rmsctl deployments show <id> | metrics <id>      # Details & Metriken
rmsctl deployments promote <id>                  # Naechste Stufe
rmsctl deployments rollback <id> [--reason X]    # Zurueckrollen
rmsctl deployments cancel <id>

rmsctl updates list | show <id>                  # OTA-Updates
rmsctl updates deployments <robot-id>            # Update-Stand eines Roboters
rmsctl updates approve <id> --approver-id X
rmsctl updates deploy <id> <robot-id>
rmsctl updates rollback <id> <robot-id> --target-version X
```

#### Modell-Checkpoint direkt hochladen

`rmsctl models upload` schickt ein trainiertes Checkpoint-Verzeichnis vom eigenen Rechner
direkt in den Modellspeicher einer RMS-Instanz und traegt es in die Model Registry ein —
ohne Hugging Face, ohne externe URL. Der Vertrag ist `docs/model-upload.md` im Server-Repo;
die Teil-Uebertragung ist dieselbe wie beim Datensatz-Upload.

```bash
# 1. Ansehen, was passieren wuerde (sendet nichts)
rmsctl --server https://neodem.example/api models upload /data/runs/pipe_g11 --version g11-step-5000

# 2. Hochladen, auf die Registrierung warten, Modell ausgeben
rmsctl --server https://neodem.example/api models upload /data/runs/pipe_g11 \
  --name "GR00T N1.7 AppleToPlate g11" --version g11-step-5000 \
  --base-model groot_n1_7 --dataset 9b7c63ca-fc76-4dd1-aed2-2dada9bdcb64 \
  --metrics final_loss=0.0264 --metrics steps=5000 \
  --eval-metrics success_rate=0.52 --eval-metrics eval_episodes=50 \
  --notes "5000 steps, lr 1e-4, batch 128, seed 42" --yes
```

```
  ◈ Upload model: GR00T N1.7 AppleToPlate g11 g11-step-5000
    │ Source         /data/runs/pipe_g11
    │ Base model     groot_n1_7
    │ Files          15 (6.9 GB)
    │                5.0 GB  model-00001-of-00002.safetensors
    │                1.9 GB  model-00002-of-00002.safetensors
  [--] Hashing 15 file(s), 6.9 GB, so the server can verify what it stored …
  [OK] Upload started: fe89db46-… (rustfs store, 32 MiB parts, 220 part(s))
    │ Uploading      ████████░░░░░░░░░░░░  41.2%  2.8 GB / 6.9 GB · 9/15 files · 90.3 MB/s · ETA 45s
  [OK] Sent 6.9 GB in 1m17s (90.3 MB/s)
  [--] The server is assembling the files and verifying checksums …
  [OK] Model registered
    │ ID             fe89db46-d44b-4edc-8794-cfe95facb611
    │ Source         uploaded directly
    │ Artifact       s3://production-models/uploads/fe89db46-…/
    │ Base model     groot_n1_7
    │ Trained on     NVIDIA AppleToPlate (9b7c63ca-…)
    │ Stored         6.9 GB in 15 file(s)
```

`<dir>` ist das Checkpoint-Verzeichnis (mit `config.json` und den Gewichten). Gesendet wird
**jede** Datei darin, mit Unterverzeichnissen — ein Checkpoint hat kein festes Layout.
`--exclude` laesst Dateien weg.

| Flag | Bedeutung |
| --- | --- |
| `--version <v>` | Pflicht: Version des Modells, z. B. `1.0.0` oder `g11-step-5000` |
| `--name` | Name (Default: Verzeichnisname). Name + Version duerfen noch nicht registriert sein |
| `--model-type` | `vla` (Default) oder `rl_policy` |
| `--base-model` | Foundation-Modell: `pi0`, `pi0_6`, `openvla`, `groot`, `groot_n1_7`, `smolvla` |
| `--dataset <id>` | Datensatz, auf dem trainiert wurde; muss auf der Instanz existieren |
| `--metrics k=v \| datei.json` | Trainingsmetriken; wiederholbar. Zahlen bleiben Zahlen, eine JSON-Datei muss ein Objekt enthalten |
| `--eval-metrics k=v \| datei.json` | Evaluierungsmetriken, gleiche Formen (z. B. `success_rate=0.52`) |
| `--notes <text>` | Freitext zum Modell (Rezept, Seed, wo evaluiert) |
| `--exclude <glob>` | Dateien weglassen, wiederholbar: `*.log`, `wandb`, `checkpoint-*/optimizer.pt`, `**/tmp` |
| `--chunk-size`, `--parallel`, `--retries`, `--resume`, `--restart`, `--no-checksum`, `--no-wait`, `--part-timeout` | Wie bei `datasets upload` |
| `--wait-timeout` | Sekunden fuer Zusammensetzen, Pruefen und Registrieren (Default 3600) |

- **Erst am Ende ein Modell:** Solange der Upload laeuft, fehlgeschlagen oder abgebrochen
  ist, steht nichts in der Registry. `rmsctl models upload-status` (ohne ID) zeigt die
  offenen Uploads. Nach dem Registrieren traegt das Modell die ID des Uploads.
- **Fortsetzen / Abbrechen:** wie beim Datensatz — denselben Befehl noch einmal ausfuehren
  bzw. `rmsctl models upload-cancel <id> --yes`. Ein registriertes Modell wird nicht
  abgebrochen, sondern mit `rmsctl models delete <id> --yes` archiviert; dabei loescht die
  Instanz die gespeicherten Dateien.
- **Name, Version und Metadaten stehen ab dem Start fest:** Ein fortgesetzter Upload behaelt,
  was beim ersten Aufruf angegeben wurde. Verlangt ein neuer Aufruf fuer dasselbe Verzeichnis
  einen anderen Namen oder eine andere Version, waehrend noch ein Upload offen ist, bricht
  rmsctl mit Exit 1 ab und nennt die Wege: gleiche Angaben (fortsetzen), `--restart`
  (verwerfen und neu hochladen) oder `--resume <id>` (unveraendert zu Ende bringen).
  Geaenderte Metriken oder Notizen brauchen ebenfalls `--restart`. Ist der gemerkte Upload
  schon fertig und ein anderes Modell, beginnt einfach ein neuer.
- **Grosse Dateien:** Eine 5-GB-Datei sind 149 Teile zu 32 MiB; das Limit liegt bei 10 000
  Teilen pro Datei. Gemessen lokal (Server + RustFS auf demselben Rechner, Quelle auf einer
  Festplatte): 6,9 GB in 15 Dateien in 173 s — 88 s Hashen, 78 s Senden (89 MB/s, begrenzt
  durch die Quellplatte, die zweimal gelesen wird), 6 s serverseitiges Zusammensetzen und
  SHA-256-Pruefung. `--no-checksum` spart den Hash-Durchlauf (die Teile bleiben einzeln
  geprueft). Der Download derselben 6,9 GB dauerte 9 s inklusive SHA-256-Vergleich.
- **Exit-Code:** 0 nur, wenn das Modell registriert ist; Ctrl-C endet mit 130.
- **`--json`:** keine Fortschrittsanzeige; am Ende steht der unveraenderte
  `GET /models/versions/<id>`-Payload auf stdout.

#### Modell herunterladen

```bash
rmsctl models files <id>                         # Was gespeichert ist
rmsctl models download <id> --out ./g11          # Alles holen
rmsctl models download <id> --out ./g11 --resume # Nach einem Abbruch fortsetzen
```

`models download` ist derselbe Mechanismus wie `datasets download` (gleiche Flags:
`--out`, `--parallel`, `--retries`, `--resume`, `--force`, `--no-checksum`,
`--stall-timeout`), nur gegen die Modell-Routen. Dateien entstehen als
`<name>.rmsctl-part` und bekommen ihren Namen erst, wenn Groesse und SHA-256 stimmen —
der SHA-256 liegt fuer jedes mit `rmsctl models upload` hochgeladene Modell vor (ausser
bei `--no-checksum`). Ohne `--out` heisst das Zielverzeichnis `./<name>-<version>`. Nur
Modelle, die direkt hochgeladen wurden, haben Dateien auf der Instanz; alle anderen
antworten `404 NO_STORED_FILES`.

### Skills, Embodiments & Prozesse

```bash
rmsctl skills list [--published]                 # VLA-Skills auflisten
rmsctl skills show <id>                          # Skill-Details
rmsctl skills for-robot <robot-id>               # Passende Skills
rmsctl skills compatible <id>                    # Passende Roboter
rmsctl skills check <id> <robot-id>              # Kompatibilitaet pruefen
rmsctl skills validate <id>
rmsctl skills publish <id> | deprecate <id> | archive <id>
rmsctl skills execute <id> --robot-id X [--prompt X]
rmsctl skills abort <id>

rmsctl embodiments list                          # Roboter-Typen auflisten
rmsctl embodiments show <tag>                    # Embodiment-Details & Config

rmsctl processes list | show <id>                # Prozess-Definitionen
rmsctl processes instances [--status X]          # Laufende Instanzen
rmsctl processes instance <id>
rmsctl processes tasks | task-queue
rmsctl processes start <id>
rmsctl processes pause <instance-id> | resume | cancel | retry
```

### Evaluation & Compliance

```bash
rmsctl eval episodes [--robot-id X] [--model-version X]
rmsctl eval success-rate [--robot-id X] [--model-version X] [--period 24h|7d|30d]
rmsctl eval episode <id>                         # Einzelne Episode
rmsctl eval errors [--robot-id X]                # Fehler-Aufschluesselung
rmsctl eval rewards                              # Reward-Model-Scores
rmsctl eval compare <versionA> <versionB> [--period 7d]
rmsctl eval run-hardware --robot-id X --skill-id Y [--episodes N]

rmsctl compliance dashboard                      # Compliance-Score
rmsctl compliance deadlines                      # Fristen
rmsctl compliance gaps [--summary]               # Offene Luecken
rmsctl compliance metrics | activity
rmsctl compliance documents                      # Ablaufende Dokumente
rmsctl compliance training [--summary]
rmsctl compliance inspections [--summary]
rmsctl compliance risk-assessments
```

### JSON-Ausgabe

Jeder Befehl unterstuetzt `--json`. Die Ausgabe ist der unveraenderte Server-Payload —
es wird nichts weggelassen und nichts hinzugefuegt:

```bash
rmsctl --json robots list
rmsctl --json datasets show <id>
rmsctl --json robots list | jq '.robots[].metadata.robotType'
rmsctl --json safety status | jq '.robots[] | select(.estop.status=="triggered")'
```

### Paginierung

Listen zeigen standardmaessig die erste Server-Seite. Wenn mehr Eintraege existieren als
angezeigt werden, sagt die Fussnote es und nennt das Flag:

```bash
$ rmsctl alerts list

  50 of 75 alert(s)
  [--] page 1 of 2 — use --page N, or --limit 75 for all
```

## VHS Tapes

Terminal-Recordings mit [VHS](https://github.com/charmbracelet/vhs):

```bash
vhs tapes/status.tape       # Flotten-Dashboard
vhs tapes/robots.tape       # Roboter-Liste & Details
vhs tapes/telemetry.tape    # Telemetrie-Ansicht
vhs tapes/pipeline.tape     # Daten- & Training-Pipeline
vhs tapes/upload.tape       # Datensatz direkt hochladen (braucht ein LeRobot-Verzeichnis)
vhs tapes/demo.tape         # Komplette Demo
```

## Stack

Rust, [clap](https://docs.rs/clap) (CLI), [reqwest](https://docs.rs/reqwest) (HTTP), [colored](https://docs.rs/colored) (Terminal-Output), [tokio](https://docs.rs/tokio) (Async), [sha2](https://docs.rs/sha2) (Pruefsummen beim Upload).
