//! O bundle de efeito que a layer executa. Por enquanto vem da variável `LINUX_RESHADE_BUNDLE`
//! (diretório gerado pelo `fx2spv`); a escolha pelo app de controle é o marco 4 do ROADMAP.

use crate::log::log;
use fx_bundle::Bundle;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Diretório configurado, se houver.
pub fn configured() -> Option<PathBuf> {
    std::env::var_os("LINUX_RESHADE_BUNDLE").map(PathBuf::from)
}

static BUNDLE: OnceLock<Option<Bundle>> = OnceLock::new();

/// O bundle carregado e validado, ou `None` (não configurado ou inválido; o motivo vai para o log,
/// uma vez só). A primeira chamada lê o disco.
pub fn get() -> Option<&'static Bundle> {
    BUNDLE
        .get_or_init(|| {
            let dir = configured()?;
            match Bundle::load(&dir) {
                Ok(bundle) => {
                    log!(
                        "bundle carregado: {} ({}x{}, {} técnica(s))",
                        dir.display(),
                        bundle.manifest.target.width,
                        bundle.manifest.target.height,
                        bundle.manifest.techniques.len()
                    );
                    Some(bundle)
                }
                Err(e) => {
                    log!("bundle {} recusado: {e}", dir.display());
                    None
                }
            }
        })
        .as_ref()
}
