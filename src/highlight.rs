//! Typst syntax highlighting for the editor, built on the official parser.

use eframe::egui::{self, Color32, FontId, text::LayoutJob};
use typst::syntax::{LinkedNode, Tag, highlight, parse};

use crate::theme::Theme;

fn color_for(tag: Tag, t: &Theme) -> Color32 {
    match tag {
        Tag::Comment => t.comment,
        Tag::Keyword | Tag::Error => t.keyword,
        Tag::String | Tag::Raw | Tag::Link => t.string,
        Tag::Function => t.function,
        Tag::Number => t.number,
        Tag::Heading => t.heading,
        Tag::Strong | Tag::Emph | Tag::ListMarker | Tag::ListTerm => t.markup,
        Tag::Label | Tag::Ref => t.label,
        Tag::MathDelimiter | Tag::MathOperator | Tag::MathGroupingParens => t.math,
        Tag::Escape => t.number,
        Tag::Interpolated => t.function,
        Tag::Operator | Tag::Punctuation => t.text,
    }
}

pub fn layout_job(text: &str, theme: &Theme, font: FontId, wrap_width: f32, line_h: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    LINE_H.with(|l| l.set(line_h));
    job.wrap.max_width = wrap_width;
    job.wrap.break_anywhere = false;
    let root = parse(text);
    let linked = LinkedNode::new(&root);
    let mut pos = 0usize;
    walk(&linked, None, theme, &font, &mut job, text, &mut pos);
    if pos < text.len() {
        push(&mut job, &text[pos..], theme.text, false, false, &font);
    }
    job
}

fn walk(node: &LinkedNode, inherited: Option<Tag>, theme: &Theme, font: &FontId, job: &mut LayoutJob, text: &str, pos: &mut usize) {
    let own = highlight(node);
    let tag = own.or(inherited);
    if node.children().len() == 0 {
        let range = node.range();
        if range.start > *pos && range.start <= text.len() {
            push(job, &text[*pos..range.start], theme.text, false, false, font);
        }
        let end = range.end.min(text.len());
        if range.start < end {
            let color = tag.map(|t| color_for(t, theme)).unwrap_or(theme.text);
            let italic = matches!(tag, Some(Tag::Emph));
            let strong = matches!(tag, Some(Tag::Strong) | Some(Tag::Heading));
            push(job, &text[range.start..end], color, italic, strong, font);
            *pos = end;
        }
        return;
    }
    // Headings/strong/emph colour their whole subtree, other tags only apply to themselves.
    let pass = match own {
        Some(Tag::Heading | Tag::Strong | Tag::Emph | Tag::Raw | Tag::Comment | Tag::String | Tag::Link) => own,
        Some(_) => inherited,
        None => inherited,
    };
    for child in node.children() {
        walk(&child, pass, theme, font, job, text, pos);
    }
}

thread_local! {
    static LINE_H: std::cell::Cell<f32> = const { std::cell::Cell::new(18.0) };
}

fn job_line_height(_job: &LayoutJob) -> f32 {
    LINE_H.with(|l| l.get())
}

fn push(job: &mut LayoutJob, s: &str, color: Color32, italic: bool, _strong: bool, font: &FontId) {
    let format = egui::TextFormat {
        font_id: font.clone(),
        color,
        italics: italic,
        line_height: Some(job_line_height(job)),
        ..Default::default()
    };
    job.append(s, 0.0, format);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::THEMES;

    fn job(text: &str) -> LayoutJob {
        layout_job(text, &THEMES[2], FontId::monospace(13.0), 400.0, 20.0)
    }

    /// The colour the highlighter gave to the first occurrence of `part`.
    fn color_of(job: &LayoutJob, part: &str) -> Color32 {
        let at = job.text.find(part).unwrap_or_else(|| panic!("{part:?} is not in the text"));
        let section = job
            .sections
            .iter()
            .find(|s| usize::from(s.byte_range.start) <= at && at < usize::from(s.byte_range.end))
            .unwrap_or_else(|| panic!("no section covers {part:?}"));
        section.format.color
    }

    /// The editor draws the job in place of the text: both must be the same, byte for byte,
    /// or the cursor would sit in the wrong place.
    #[test]
    fn every_byte_is_laid_out_exactly_once() {
        for text in [
            "",
            "\n",
            "= Überschrift\n\nText mit *fett*, _kursiv_ und `roh`. // Kommentar\n",
            "#let x = (1, \"zwei\", 3.5em)\n#if x.len() > 2 [ja] else [nein]\n$ sum_(i=0)^n i^2 $ <marke> @marke",
            // Unfinished and broken input, as it is while typing.
            "#let s = \"offen",
            "$ x^",
            "*fett ohne Ende\n/* Kommentar ohne Ende",
            "#(",
            "Zeile\r\nmit Windows-Umbruch\r\n\tund Tab — „Zitat“ 🙂",
            "]]] }} ))",
        ] {
            let job = job(text);
            assert_eq!(job.text, text);
            let mut at = 0;
            for section in &job.sections {
                assert_eq!(usize::from(section.byte_range.start), at, "sections are gapless in {text:?}");
                at = usize::from(section.byte_range.end);
            }
            assert_eq!(at, text.len(), "sections end with {text:?}");
        }
    }

    #[test]
    fn syntax_gets_its_colours() {
        let t = &THEMES[2];
        let job = job("= Titel\n#let zahl = 42 // Hinweis\n#text(\"Wort\") einfach <marke>");
        assert_eq!(color_of(&job, "Titel"), t.heading);
        assert_eq!(color_of(&job, "let"), t.keyword);
        assert_eq!(color_of(&job, "42"), t.number);
        assert_eq!(color_of(&job, "// Hinweis"), t.comment);
        assert_eq!(color_of(&job, "\"Wort\""), t.string);
        assert_eq!(color_of(&job, "text"), t.function);
        assert_eq!(color_of(&job, "einfach"), t.text);
        assert_eq!(color_of(&job, "<marke>"), t.label);
    }

    #[test]
    fn lines_get_the_editor_line_height() {
        let job = layout_job("a\nb", &THEMES[0], FontId::monospace(13.0), f32::INFINITY, 23.0);
        assert!(job.sections.iter().all(|s| s.format.line_height == Some(23.0)));
    }
}
