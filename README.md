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

## Befehle

### Flotte

```bash
rmsctl status                                    # Flotten-Dashboard
rmsctl robots list                               # Alle Roboter auflisten
rmsctl robots show <id>                          # Roboter-Details
rmsctl robots telemetry <id>                     # Telemetrie (CPU, Joints, Sensoren)
rmsctl robots register <url>                     # Roboter registrieren
rmsctl robots remove <id>                        # Roboter entfernen
rmsctl robots command <id> <type> [--payload {}] # Befehl senden
```

### Alerts

```bash
rmsctl alerts list                               # Alle Alerts
rmsctl alerts list --active                      # Nur unbestaetigte
rmsctl alerts list --severity critical           # Nach Schweregrad filtern
rmsctl alerts ack <id>                           # Alert bestaetigen
```

### Daten & Training

```bash
rmsctl datasets list                             # Datensaetze auflisten
rmsctl datasets show <id>                        # Datensatz-Details
rmsctl datasets episodes <id>                    # Episoden eines Datensatzes

rmsctl training list                             # Trainingsjobs auflisten
rmsctl training show <id>                        # Job-Details & Fortschritt
rmsctl training active                           # Laufende Jobs
rmsctl training queue                            # Warteschlangen-Status
```

### Modelle & Deployment

```bash
rmsctl models list                               # Modell-Registry
rmsctl models show <name>                        # Modell-Details & Versionen
rmsctl models compare <runA> <runB> [--metrics]  # Runs vergleichen

rmsctl deployments list                          # Deployments auflisten
rmsctl deployments show <id>                     # Deployment-Details
```

### Skills & Embodiments

```bash
rmsctl skills list                               # VLA-Skills auflisten
rmsctl skills show <id>                          # Skill-Details

rmsctl embodiments list                          # Roboter-Typen auflisten
rmsctl embodiments show <tag>                    # Embodiment-Details & Config
```

### Evaluation

```bash
rmsctl eval episodes [--robot-id X] [--model-version X]  # Evaluierungs-Episoden
rmsctl eval success-rate [--robot-id X] [--model-version X]  # Erfolgsrate
rmsctl eval compare <versionA> <versionB>                 # A/B-Vergleich
```

### JSON-Ausgabe

Jeder Befehl unterstuetzt `--json` fuer maschinenlesbare Ausgabe:

```bash
rmsctl --json robots list
rmsctl --json datasets show <id>
rmsctl --json status | jq '.robots.robots[].name'
```

## VHS Tapes

Terminal-Recordings mit [VHS](https://github.com/charmbracelet/vhs):

```bash
vhs tapes/status.tape       # Flotten-Dashboard
vhs tapes/robots.tape       # Roboter-Liste & Details
vhs tapes/telemetry.tape    # Telemetrie-Ansicht
vhs tapes/pipeline.tape     # Daten- & Training-Pipeline
vhs tapes/demo.tape         # Komplette Demo
```

## Stack

Rust, [clap](https://docs.rs/clap) (CLI), [reqwest](https://docs.rs/reqwest) (HTTP), [colored](https://docs.rs/colored) (Terminal-Output), [tokio](https://docs.rs/tokio) (Async).
