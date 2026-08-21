//! src/domain/subscriber_name.rs
use unicode_segmentation::UnicodeSegmentation;

// 元组结构体
#[derive(Debug)]
pub struct SubscriberName(String);

impl SubscriberName {
    /// 如果输入满足我们对订阅者姓名的所有验证约束，返回一个'SubscriberName'实例
    /// 否则会抛出一个panic!
    pub fn parse(s: String) -> Result<Self, String> {
        // '.trim()'返回一个s的视图，不包括尾随的类似于空格的字符
        let is_empty_or_whitespace = s.trim().is_empty();

        // Unicode标准将一个音素定义为”用户感知“的字符
        // 'graphemes'返回一个迭代器，用于遍历所有的音素
        // ’true'指定我们使用扩展音素定义集
        let is_too_long = s.graphemes(true).count() > 256;

        // 遍历输入中的所有字符，检查是否包含禁用数组中的任何字符匹配
        let forbidden_characters = ['/', '(', ')', '"', '<', '>', '\\', '{', '}'];
        let contains_forbidden_characters = s.chars().any(|c| forbidden_characters.contains(&c));

        if is_empty_or_whitespace || is_too_long || contains_forbidden_characters {
            Err(format!("{} is not a valid SubscriberName name.", s))
        } else {
            Ok(Self(s))
        }
    }
}

impl AsRef<str> for SubscriberName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}



#[cfg(test)]
mod tests {
    use crate::domain::subscriber_name::SubscriberName;
    use claim::{assert_err, assert_ok};

    #[test]
    fn a_256_grapheme_long_name_is_valid() {
        let name = "ë".repeat(256);
        assert_ok!(SubscriberName::parse(name));
    }

    #[test]
    fn a_name_longer_than_256_graphemes_is_rejected() {
        let name = "a".repeat(257);
        assert_err!(SubscriberName::parse(name));
    }

    #[test]
    fn whitespace_only_names_are_rejected() {
        let name = "".to_string();
        assert_err!(SubscriberName::parse(name));
    }

    #[test]
    fn empty_string_is_rejected() {
        let name = "".to_string();
        assert_err!(SubscriberName::parse(name));
    }

    #[test]
    fn names_containing_an_invalid_character_are_rejected() {
        for name in &['/', '(', ')', '"', '<', '>', '\\', '{', '}'] {
            let name = name.to_string();
            assert_err!(SubscriberName::parse(name));
        }
    }

    #[test]
    fn a_valid_name_is_parsed_successfully() {
        let name = "Ursula Le Guin".to_string();
        assert_ok!(SubscriberName::parse(name));
    }
}
