//! O KDE guarda a tecla de cada atalho pelo id, e o portal só usa a tecla sugerida na PRIMEIRA vez que vê o id: registrar de novo
//! com outra sugestão mantém a antiga. Para a pessoa poder trocar a tecla, o atalho é antes esquecido pelo `KGlobalAccel` (o
//! serviço de atalhos do KDE), e o registro seguinte volta a valer a sugestão. Fora do KDE não há nada a esquecer.

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedObjectPath;

const DESTINATION: &str = "org.kde.kglobalaccel";
const ROOT: &str = "/kglobalaccel";

/// Esquece, em todos os componentes do KDE, os atalhos com estes ids. Devolve quantos esqueceu; sem o KDE, 0.
pub fn forget(ids: &[&str]) -> usize {
    let Ok(connection) = Connection::session() else { return 0 };
    let Ok(root) = Proxy::new(&connection, DESTINATION, ROOT, "org.kde.KGlobalAccel") else { return 0 };
    let Ok(components) = root.call::<_, _, Vec<OwnedObjectPath>>("allComponents", &()) else { return 0 };
    let mut forgotten = 0;
    for path in components {
        let Ok(component) = Proxy::new(&connection, DESTINATION, path.as_str(), "org.kde.kglobalaccel.Component") else { continue };
        let Ok(names) = component.call::<_, _, Vec<String>>("shortcutNames", &()) else { continue };
        let Ok(unique) = component.get_property::<String>("uniqueName") else { continue };
        for id in ids.iter().filter(|id| names.iter().any(|n| n == *id)) {
            if root.call::<_, _, bool>("unregister", &(unique.as_str(), *id)).unwrap_or(false) {
                forgotten += 1;
            }
        }
    }
    forgotten
}

/// Guarda as teclas de cada atalho enquanto ele está solto (ver `set_grabbed`).
static RELEASED: std::sync::Mutex<Vec<(String, Vec<i32>)>> = std::sync::Mutex::new(Vec::new());

/// Solta ou retoma as teclas (`grabbed = false` solta) dos atalhos `ids` do componente `component` SEM esquecê-los: continuam registrados e
/// confirmados, mas o KDE deixa de tomar a tecla dos outros programas. Devolve se conseguiu (sem o KDE, `false`).
pub fn set_grabbed(component: &str, ids: &[&str], grabbed: bool) -> bool {
    let Ok(connection) = Connection::session() else { return false };
    let Ok(root) = Proxy::new(&connection, DESTINATION, ROOT, "org.kde.KGlobalAccel") else { return false };
    let mut released = RELEASED.lock().unwrap_or_else(|e| e.into_inner());
    let mut done = true;
    for id in ids {
        let action: Vec<&str> = vec![component, id, "", ""];
        const NO_AUTOLOADING: u32 = 4;
        if grabbed {
            let Some(at) = released.iter().position(|(i, _)| i == id) else { continue };
            let keys = released.remove(at).1;
            done &= root.call::<_, _, Vec<i32>>("setShortcut", &(&action, keys, NO_AUTOLOADING)).is_ok();
        } else if !released.iter().any(|(i, _)| i == id) {
            let keys = match root.call::<_, _, Vec<i32>>("shortcut", &(&action,)) { Ok(k) => k, Err(e) => { eprintln!("shortcut: {e}"); done = false; continue } };
            if keys.is_empty() {
                continue;
            }
            released.push((id.to_string(), keys));
            let r = root.call::<_, _, Vec<i32>>("setShortcut", &(&action, Vec::<i32>::new(), NO_AUTOLOADING)); if let Err(e) = &r { eprintln!("setShortcut: {e}"); } done &= r.is_ok();
        }
    }
    done
}
