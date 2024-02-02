use core::fmt::Write;

pub struct HtmlEscape<'a> {
    src: &'a str,
}

impl<'a> HtmlEscape<'a> {
    pub fn from_str(src: &'a str) -> HtmlEscape<'a> {
        HtmlEscape { src }
    }
}

impl<'a> core::fmt::Display for HtmlEscape<'a> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for ch in self.src.chars() {
            match ch {
                '"' => f.write_str("&quot;")?,
                '<' => f.write_str("&lt;")?,
                '>' => f.write_str("&gt;")?,
                '&' => f.write_str("&amp;")?,
                ch => f.write_char(ch)?,
            }
        }

        Ok(())
    }
}

pub struct UriDecode<'a> {
    src: &'a str,
}

impl<'a> UriDecode<'a> {
    pub fn from_str(src: &'a str) -> UriDecode<'a> {
        UriDecode { src }
    }

    fn ch_to_hex(ch: char) -> Option<u8> {
        match ch.to_ascii_uppercase() {
            '0' => Some(0),
            '1' => Some(1),
            '2' => Some(2),
            '3' => Some(3),
            '4' => Some(4),
            '5' => Some(5),
            '6' => Some(6),
            '7' => Some(7),
            '8' => Some(8),
            '9' => Some(9),
            'A' => Some(10),
            'B' => Some(11),
            'C' => Some(12),
            'D' => Some(13),
            'E' => Some(14),
            'F' => Some(15),
            _ => None,
        }
    }
}

impl<'a> core::fmt::Display for UriDecode<'a> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut chars = self.src.chars();

        while let Some(ch) = chars.next() {
            match ch {
                '+' => f.write_char(' ')?,
                '%' => {
                    let high = UriDecode::ch_to_hex(chars.next().unwrap()).unwrap();
                    let low = UriDecode::ch_to_hex(chars.next().unwrap()).unwrap();

                    let unescaped = high << 4 | low;
                    f.write_char(unescaped.into())?;
                }
                ch => f.write_char(ch)?,
            }
        }

        Ok(())
    }
}
