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
