pub(crate) const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
pub(crate) const UTF16LE_BOM: [u8; 2] = [0xFF, 0xFE];
pub(crate) const UTF16BE_BOM: [u8; 2] = [0xFE, 0xFF];

pub(crate) fn has_bom(bytes: &[u8]) -> bool {
    bytes.starts_with(&UTF8_BOM)
        || bytes.starts_with(&UTF16LE_BOM)
        || bytes.starts_with(&UTF16BE_BOM)
}
