//! Whisper `initial_prompt` / hotword bias. Does not replace the scientific parser.

use sciwhisper_core::Domain;

const CHEMISTRY: &str = "\
Научная диктовка по химии. \
гидроксид меди два, гидроксид железа три, оксид меди два, оксид железа три, сульфат ион два минус, \
ион меди два плюс, аш два эс о четыре, калий марганец о четыре, \
цинк плюс два аш хлор, превращается в, медный купорос, вода, углекислый газ.";

const MATH: &str = "\
Научная диктовка по математике. \
икс в квадрате, игрек, эн большое, эн малое, начало дроби, числитель, знаменатель, \
конец дроби, корень из, сумма от ка равно нулю до бесконечности, \
интеграл от нуля до единицы по икс, факториал четырёх ка.";

const PHYSICS: &str = "\
Научная диктовка по физике. \
дельта же, дельта аш, лямбда, ню греческая, вектор эф, \
девять целых восемьдесят одна метра на секунду в квадрате, нанометра.";

pub fn for_domain(domain: Domain) -> String {
    match domain {
        Domain::Chemistry => CHEMISTRY.to_string(),
        Domain::Mathematics => MATH.to_string(),
        Domain::Physics => PHYSICS.to_string(),
        Domain::Auto | Domain::Plain => combined(),
    }
}

pub fn combined() -> String {
    format!("{CHEMISTRY} {MATH} {PHYSICS}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chemistry_prompt_contains_copper_hydroxide() {
        assert!(for_domain(Domain::Chemistry).contains("гидроксид меди два"));
    }
}
