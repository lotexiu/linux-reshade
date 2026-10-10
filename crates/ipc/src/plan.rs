//! O plano de um jogo: a lista ordenada de técnicas que a layer executa, cada uma com o bloco de uniforms
//! pronto (os valores do preset sobre os padrões do efeito). Quem monta é o controlador; a layer só lê.
//!
//! No canal de um jogo vale antes da escolha única de efeito: com um plano publicado (mesmo vazio) a layer
//! o segue; sem plano, cai na escolha e nos parâmetros de antes.
//!
//! Formato do blob (inteiros em little-endian):
//!
//! ```text
//! u32 quantidade de etapas
//! por etapa:  u16 n + n bytes   modelo de caminho do bundle (com {W} e {H})
//!             u16 n + n bytes   nome da técnica ("" = a primeira)
//!             u64               params_id do efeito (a layer só aplica o bloco ao bundle que tem este id)
//!             u32 n + n bytes   imagem do buffer de uniforms
//! ```

/// Tamanho máximo do plano publicado.
pub const MAX_PLAN_BYTES: usize = 131_072;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanStep {
    /// Modelo de caminho do bundle, como na escolha única (`<cache>/Sepia-{W}x{H}`).
    pub template: String,
    /// A técnica dentro do bundle; vazio = a primeira.
    pub technique: String,
    /// `fx_bundle::Bundle::params_id` do efeito.
    pub params_id: u64,
    /// A imagem do buffer de uniforms (vazia = padrões do manifesto).
    pub uniforms: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    pub steps: Vec<PlanStep>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PlanError {
    /// Um texto passa de 65535 bytes ou um bloco passa de 4 GiB.
    FieldTooLong,
}

impl Plan {
    pub fn encode(&self) -> Result<Vec<u8>, PlanError> {
        let mut out = Vec::new();
        out.extend_from_slice(&(self.steps.len() as u32).to_le_bytes());
        for step in &self.steps {
            for text in [&step.template, &step.technique] {
                let n = u16::try_from(text.len()).map_err(|_| PlanError::FieldTooLong)?;
                out.extend_from_slice(&n.to_le_bytes());
                out.extend_from_slice(text.as_bytes());
            }
            out.extend_from_slice(&step.params_id.to_le_bytes());
            let n = u32::try_from(step.uniforms.len()).map_err(|_| PlanError::FieldTooLong)?;
            out.extend_from_slice(&n.to_le_bytes());
            out.extend_from_slice(&step.uniforms);
        }
        Ok(out)
    }

    /// `None` se os bytes não são um plano inteiro e bem formado (a layer então ignora esta publicação).
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = Reader(bytes);
        let count = r.u32()? as usize;
        // Cada etapa ocupa ao menos 16 bytes: um contador absurdo não aloca nada.
        if count > bytes.len() / 16 {
            return None;
        }
        let mut steps = Vec::with_capacity(count);
        for _ in 0..count {
            let template = r.text()?;
            let technique = r.text()?;
            let params_id = r.u64()?;
            let n = r.u32()? as usize;
            let uniforms = r.take(n)?.to_vec();
            steps.push(PlanStep { template, technique, params_id, uniforms });
        }
        r.0.is_empty().then_some(Self { steps })
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, tail) = self.0.split_at_checked(n)?;
        self.0 = tail;
        Some(head)
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn text(&mut self) -> Option<String> {
        let n = self.u16()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Plan {
        Plan {
            steps: vec![
                PlanStep { template: "/c/Sepia-{W}x{H}".into(), technique: "Sepia".into(), params_id: 0xDEAD_BEEF_0123_4567, uniforms: vec![1, 2, 3, 4, 5] },
                PlanStep { template: "/c/Çor-{W}x{H}".into(), technique: String::new(), params_id: 9, uniforms: vec![] },
            ],
        }
    }

    #[test]
    fn a_plan_round_trips() {
        let plan = sample();
        assert_eq!(Plan::decode(&plan.encode().unwrap()), Some(plan));
        let empty = Plan::default();
        assert_eq!(Plan::decode(&empty.encode().unwrap()), Some(empty));
    }

    #[test]
    fn truncated_or_padded_input_is_refused_never_a_panic() {
        let bytes = sample().encode().unwrap();
        for cut in 0..bytes.len() {
            assert_eq!(Plan::decode(&bytes[..cut]), None, "cortado em {cut}");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert_eq!(Plan::decode(&longer), None, "sobra no fim");
        assert_eq!(Plan::decode(&[]), None);
    }

    #[test]
    fn absurd_counts_and_lengths_are_refused() {
        assert_eq!(Plan::decode(&u32::MAX.to_le_bytes()), None);
        let mut bytes = 1u32.to_le_bytes().to_vec();
        bytes.extend_from_slice(&u16::MAX.to_le_bytes()); // texto de 65535 bytes que não existem
        assert_eq!(Plan::decode(&bytes), None);
        // Texto que não é UTF-8.
        let mut bad = 1u32.to_le_bytes().to_vec();
        bad.extend_from_slice(&[1, 0, 0xFF, 0, 0]);
        bad.extend_from_slice(&[0; 12]);
        assert_eq!(Plan::decode(&bad), None);
    }

    #[test]
    fn a_text_field_over_64k_cannot_be_encoded() {
        let plan = Plan { steps: vec![PlanStep { template: "x".repeat(70_000), technique: String::new(), params_id: 0, uniforms: vec![] }] };
        assert_eq!(plan.encode(), Err(PlanError::FieldTooLong));
    }
}
