use std::borrow::Cow;

use icu_casemap::CaseMapper;
use icu_properties::{
    props::{GeneralCategory, Lowercase, Uppercase},
    CodePointMapData, CodePointSetData,
};
use unicode_normalization::UnicodeNormalization;

pub(crate) fn encode(text: &[u8]) -> Cow<'_, [u8]> {
    let valid = std::str::from_utf8(text).ok().or_else(|| {
        (1..=text.len().min(3))
            .find_map(|trim| std::str::from_utf8(&text[..text.len() - trim]).ok())
    });
    let Some(valid) = valid else {
        return Cow::Borrowed(text);
    };
    let lowercase = CodePointSetData::new::<Lowercase>();
    let uppercase = CodePointSetData::new::<Uppercase>();
    let categories = CodePointMapData::<GeneralCategory>::new();
    let mut all_upper = false;
    for letter in valid.chars() {
        if lowercase.contains(letter) || categories.get(letter) == GeneralCategory::TitlecaseLetter
        {
            all_upper = false;
            break;
        }
        if uppercase.contains(letter) {
            all_upper = true;
        }
    }
    let lowered = if all_upper {
        CaseMapper::new().lowercase_to_string(valid, &"und".parse().expect("valid root locale"))
    } else {
        Cow::Borrowed(valid)
    };
    Cow::Owned(
        lowered
            .chars()
            .stream_safe()
            .nfc()
            .collect::<String>()
            .into_bytes(),
    )
}
