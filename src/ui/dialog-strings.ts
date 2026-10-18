// Dialog'un tek metni. Tek kaynak burasıdır; `app/strings` (ana paket) bunu içe aktarmaz ve `t.server.close` kaldırıldı.
// Neden ayrı modül: Dialog birden çok tembel girişin paylaştığı ortak parçadır ve Rolldown ortak parçadan giriş (index)
// parçasına içe aktarma kurmaz; ortak parça `app/strings` gibi ana pakette de yaşayan bir modülü içe aktarırsa o modül
// ayrı bir parçaya bölünür ve index.html önyüklemesine girer. Bu yaprak modülü yalnız tembel parçalar içe aktarır.
export const dialogStrings = { close: "Kapat", cancel: "Vazgeç" };
