//! Built-in project templates (all self-contained, no downloads needed).

pub struct Template {
    pub name: &'static str,
    pub description: &'static str,
    pub category: &'static str,
    pub source: &'static str,
}

pub const CATEGORIES: [&str; 11] = [
    "Book", "Cv", "Flyer", "Fun", "Integration", "Layout", "Office", "Paper", "Presentation", "Report", "Utility",
];

pub const SIGNATURE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 240 70" width="240" height="70">
<path d="M8 48 C 20 10, 34 8, 30 40 S 44 62, 56 34 S 70 18, 74 44 C 78 58, 92 30, 100 36 C 108 42, 104 56, 118 46 C 130 36, 134 22, 146 40 C 152 50, 162 52, 172 38 C 180 28, 188 34, 196 42 C 206 52, 220 40, 232 30" fill="none" stroke="#1d2433" stroke-width="2.4" stroke-linecap="round"/>
<path d="M60 58 C 110 54, 170 52, 226 50" fill="none" stroke="#1d2433" stroke-width="1.4" stroke-linecap="round"/>
</svg>"##;

pub const INVOICE: &str = r##"// Rechnung – ein Beispielprojekt
#let rechnung = (
  nummer: "2026-001",
  datum: datetime(year: 2026, month: 11, day: 7),
  posten: (
    (
      beschreibung: "Konzept und Gestaltung der neuen Broschüre, inklusive zwei Korrekturrunden",
      preis: 480,
    ),
    (
      beschreibung: "Satz und Druckvorbereitung",
      preis: 220.5,
    ),
  ),
)
// Absender
#let absender = (
  name: "Mara Beispiel",
  strasse: "Lindenweg 4",
  plz: "10115",
  ort: "Berlin",
  steuernummer: "12/345/67890",
)
// Empfänger
#let empfaenger = (
  name: "Erika Mustermann",
  strasse: "Musterallee 1",
  plz: "12345",
  ort: "Musterstadt",
)
// Bankverbindung
#let bank = (
  inhaber: "Mara Beispiel",
  institut: "Beispielbank",
  iban: "DE00 1234 5678 9012 3456 78",
  bic: "BEISDEXX",
)

#set page(paper: "a4", margin: (x: 2.4cm, top: 3.2cm, bottom: 2.5cm))
#set text(font: "Libertinus Serif", size: 11pt, lang: "de")
#show heading: set text(size: 16pt)

#let euro(x) = {
  let cents = int(calc.round(x * 100))
  let c = calc.rem(cents, 100)
  str(calc.quo(cents, 100)) + "," + (if c < 10 { "0" } else { "" }) + str(c) + "€"
}
#let summe = rechnung.posten.map(p => p.preis).sum()

#text(size: 9pt, smallcaps[*#absender.name* • #absender.strasse • #absender.plz #absender.ort])

#empfaenger.name \
#empfaenger.strasse \
#empfaenger.plz #empfaenger.ort

#v(3.5em)
#grid(
  columns: (1fr, auto),
  align: (left + bottom, right + bottom),
  heading[Rechnung \##rechnung.nummer],
  [#absender.ort, *#rechnung.datum.display("[day].[month].[year]")*],
)

#table(
  columns: (auto, 1fr, auto),
  stroke: none,
  align: (right, left, right),
  table.hline(stroke: 0.6pt),
  [*Pos.*], [*Beschreibung*], [*Preis*],
  table.hline(stroke: 0.4pt),
  ..rechnung.posten.enumerate().map(((i, p)) => ([#(i + 1).], p.beschreibung, euro(p.preis))).flatten(),
  table.hline(stroke: 0.6pt),
)
#align(right)[Summe: #box(stroke: (bottom: 0.5pt), inset: (bottom: 3pt), euro(summe))]
#v(0.6em)
#align(right)[*Gesamt:* #box(stroke: (bottom: 0.8pt), inset: (bottom: 3pt), strong(euro(summe)))]

#v(2em)
#text(size: 9.5pt)[
  Vielen Dank für die Zusammenarbeit. Bitte überweisen Sie den Rechnungsbetrag
  innerhalb von 14 Tagen ohne Abzug auf das unten genannte Konto. Gemäß § 19 UStG
  wird keine Umsatzsteuer berechnet.
]

#v(1em)
Kontoinhaberin: #bank.inhaber \
Kreditinstitut: #bank.institut \
IBAN: *#bank.iban* \
BIC: #bank.bic

Steuernummer: #absender.steuernummer

Mit freundlichen Grüßen \
#image("beispiel-unterschrift.svg", width: 4.5cm) \
#absender.name
"##;

pub const TEMPLATES: &[Template] = &[
    Template {
        name: "Leeres Dokument",
        description: "Eine leere main.typ, um bei null anzufangen.",
        category: "Utility",
        source: "",
    },
    Template {
        name: "Rechnung",
        description: "Eine schlichte Rechnung mit Posten, Summe und Bankverbindung.",
        category: "Office",
        source: INVOICE,
    },
    Template {
        name: "Brief",
        description: "Ein Geschäftsbrief mit Absender, Anschrift und Betreff.",
        category: "Office",
        source: r##"#set page(paper: "a4", margin: (x: 2.5cm, y: 2.5cm))
#set text(font: "Libertinus Serif", size: 11pt, lang: "de")

#align(right)[
  *Max Muster* \
  Hauptstraße 12 \
  12345 Musterstadt
]

#v(2em)
Firma Beispiel GmbH \
Postfach 100 \
54321 Beispielstadt

#v(2em)
#align(right)[Musterstadt, #datetime.today().display("[day].[month].[year]")]

*Betreff: Ihre Anfrage*

Sehr geehrte Damen und Herren,

vielen Dank für Ihre Nachricht. Hier steht der Text Ihres Briefes. Typst
bricht die Zeilen automatisch um und setzt Absätze sauber.

Mit freundlichen Grüßen

#v(2em)
Max Muster
"##,
    },
    Template {
        name: "Lebenslauf",
        description: "Ein klarer, einspaltiger Lebenslauf.",
        category: "Cv",
        source: r##"#set page(paper: "a4", margin: 2cm)
#set text(font: "Libertinus Serif", size: 10.5pt, lang: "de")
#show heading.where(level: 2): it => [
  #v(0.6em)
  #text(fill: rgb("#1f5aa6"), smallcaps(it.body))
  #v(-0.6em)
  #line(length: 100%, stroke: 0.5pt + rgb("#1f5aa6"))
]
#let eintrag(titel, ort, zeit, body) = [
  #grid(columns: (1fr, auto), [*#titel*], [#zeit])
  #emph(ort) \
  #body
]

#text(size: 22pt, fill: rgb("#1f5aa6"))[*Alex Beispiel*] \
alex\@beispiel.de • +49 123 456789 • Berlin

== Berufserfahrung
#eintrag("Softwareentwicklerin", "Beispiel AG, Berlin", "2022 – heute")[
  - Entwicklung einer Plattform für Dokumente
  - Leitung eines Teams aus vier Personen
]
#eintrag("Werkstudentin", "Muster GmbH, Hamburg", "2019 – 2022")[
  - Automatisierung von Tests
]

== Ausbildung
#eintrag("M.Sc. Informatik", "Universität Musterstadt", "2017 – 2022")[]

== Kenntnisse
Rust, Python, Typst, Datenbanken, Teamarbeit
"##,
    },
    Template {
        name: "Wissenschaftlicher Artikel",
        description: "Zweispaltiger Artikel mit Titel, Zusammenfassung und Abschnitten.",
        category: "Paper",
        source: r##"#set page(paper: "a4", margin: (x: 1.8cm, y: 2cm), columns: 2)
#set text(font: "New Computer Modern", size: 10pt, lang: "de")
#set par(justify: true)
#set heading(numbering: "1.")

#place(top + center, float: true, scope: "parent")[
  #text(size: 17pt)[*Ein Beispiel für einen Artikel*]
  #v(0.5em)
  Erika Muster #h(2em) Max Beispiel \
  #text(size: 9pt)[Universität Musterstadt]
  #v(1em)
  #block(width: 85%)[#align(left)[*Zusammenfassung* — #lorem(50)]]
  #v(1em)
]

= Einleitung
#lorem(80)

= Methode
#lorem(60)
$ E = m c^2 $
#lorem(40)

= Ergebnisse
#lorem(90)
"##,
    },
    Template {
        name: "Präsentation",
        description: "Folien im 16:9-Format mit großem Titel.",
        category: "Presentation",
        source: r##"#set page(width: 25.4cm, height: 14.29cm, margin: 1.6cm, fill: rgb("#14213d"))
#set text(font: "New Computer Modern", size: 22pt, fill: white, lang: "de")
#let folie(titel, body) = {
  pagebreak(weak: true)
  text(size: 30pt, fill: rgb("#fca311"))[*#titel*]
  v(0.8em)
  body
}

#align(center + horizon)[
  #text(size: 40pt)[*Mein Vortrag*] \
  #text(size: 20pt)[Ein kurzer Untertitel]
]

#folie("Überblick")[
  - Erster Punkt
  - Zweiter Punkt
  - Dritter Punkt
]

#folie("Formeln")[
  $ integral_0^1 x^2 dif x = 1/3 $
]
"##,
    },
    Template {
        name: "Bericht",
        description: "Bericht mit Titelseite, Inhaltsverzeichnis und Kapiteln.",
        category: "Report",
        source: r##"#set page(paper: "a4", margin: 2.5cm, numbering: "1")
#set text(font: "Libertinus Serif", size: 11pt, lang: "de")
#set heading(numbering: "1.1")
#set par(justify: true)

#align(center + horizon)[
  #text(size: 26pt)[*Projektbericht*] \
  #v(0.5em)
  #text(size: 14pt)[Abschlussbericht 2026]
]
#pagebreak()
#outline()
#pagebreak()

= Einleitung
#lorem(120)

== Ziele
#lorem(60)

= Umsetzung
#lorem(150)

= Fazit
#lorem(80)
"##,
    },
    Template {
        name: "Checkliste",
        description: "Eine Checkliste mit Kästchen zum Abhaken.",
        category: "Utility",
        source: r##"#set page(paper: "a5", margin: 1.5cm)
#set text(font: "DejaVu Sans Mono", size: 10pt)
#let punkt(was, status) = grid(
  columns: (auto, 1fr, auto),
  column-gutter: 4pt,
  was, box(width: 1fr, repeat[.]), status,
)

#align(center, block(stroke: 1pt, inset: 6pt, width: 100%)[*CHECKLISTE*])
#align(center, block(stroke: 1pt, inset: 6pt, width: 100%)[VOR DER ABREISE])

#punkt[Fenster][GESCHLOSSEN]
#punkt[Herd][AUS]
#punkt[Pflanzen][GEGOSSEN]
#punkt[Schlüssel][DABEI]
#punkt[Ausweis][DABEI]
"##,
    },
    Template {
        name: "Flyer",
        description: "Ein farbiger Flyer für Veranstaltungen.",
        category: "Flyer",
        source: r##"#set page(paper: "a5", margin: 0pt, fill: gradient.linear(rgb("#0f766e"), rgb("#164e63"), angle: 90deg))
#set text(font: "New Computer Modern", fill: white, lang: "de")

#pad(2cm)[
  #text(size: 14pt)[SAMSTAG · 14. MÄRZ]
  #v(1cm)
  #text(size: 44pt)[*Sommer\ fest*]
  #v(1cm)
  #text(size: 13pt)[Musik, Essen und gute Gesellschaft im Hof der alten Fabrik.]
  #v(1fr)
  #line(length: 100%, stroke: white)
  ab 16 Uhr · Eintritt frei
]
"##,
    },
    Template {
        name: "Buch",
        description: "Ein Buch mit Kapiteln und Kopfzeile.",
        category: "Book",
        source: r##"#set page(width: 13cm, height: 20cm, margin: (inside: 2cm, outside: 1.5cm, y: 2cm), numbering: "1")
#set text(font: "Libertinus Serif", size: 10.5pt, lang: "de")
#set par(justify: true, first-line-indent: 1em)
#show heading.where(level: 1): it => {
  pagebreak(weak: true)
  v(3cm)
  text(size: 20pt, it.body)
  v(1cm)
}

#align(center + horizon)[#text(size: 28pt)[*Der Titel*] \ #v(1em) Eine Erzählung]

= Erstes Kapitel
#lorem(200)

#lorem(150)

= Zweites Kapitel
#lorem(220)
"##,
    },
    Template {
        name: "Formelsammlung",
        description: "Mathematische Formeln, schön gesetzt.",
        category: "Fun",
        source: r##"#set page(paper: "a5", margin: 1.5cm)
#set text(font: "New Computer Modern", lang: "de")
#set heading(numbering: "1.")

= Analysis
$ f'(x) = lim_(h -> 0) (f(x + h) - f(x)) / h $
$ integral_a^b f(x) dif x = F(b) - F(a) $

= Reihen
$ sum_(k=1)^n k = (n(n+1)) / 2 $
$ e^x = sum_(n=0)^oo x^n / n! $

= Lineare Algebra
$ mat(a, b; c, d)^(-1) = 1 / (a d - b c) mat(d, -b; -c, a) $
"##,
    },
];
