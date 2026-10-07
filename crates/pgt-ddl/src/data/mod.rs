//! Veri düzenleyici (pgAdmin "View/Edit Data" karşılığı) arka ucu: kaynak tanımı, SELECT üretimi, değişiklik
//! planı ve kapılı tek transaction'lı uygulama. İstemci SQL göndermez (tek istisna doğrulanan filtre gövdesi);
//! ifadeleri arka uç üretir. Hücre/anahtar/sürüm değerleri ve adlar günlüğe ya da `Debug`'a girmez.

pub mod model;
pub mod plan;
pub mod select;
pub mod source;
