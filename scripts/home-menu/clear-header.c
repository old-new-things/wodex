/*
 * Wodex build helper, WTFPL-2.0.
 *
 * Decrypts only headers produced by this build's fixed public test key.
 * This program has no key-file input and never handles console key material.
 * mbedTLS is supplied by the pinned, external build tool.
 */
#include <stdio.h>
#include <stdint.h>
#include <mbedtls/aes.h>

int main(void) {
  unsigned char input[0xc00], output[0xc00], key[32];
  mbedtls_aes_context cipher, tweak_cipher;
  if (fread(input, 1, sizeof input, stdin) != sizeof input || fgetc(stdin) != EOF) {
    fputs("Expected exactly one synthetic NCA header.\n", stderr);
    return 1;
  }
  for (unsigned i = 0; i < sizeof key; ++i) key[i] = (unsigned char)i;
  mbedtls_aes_init(&cipher);
  mbedtls_aes_init(&tweak_cipher);
  int result = mbedtls_aes_setkey_dec(&cipher, key, 128);
  if (!result) result = mbedtls_aes_setkey_enc(&tweak_cipher, key + 16, 128);
  for (unsigned sector = 0; sector < 6 && !result; ++sector) {
    unsigned char tweak[16] = {0};
    tweak[15] = (unsigned char)sector;
    result = mbedtls_aes_crypt_xts(&cipher, &tweak_cipher, MBEDTLS_AES_DECRYPT, 512, tweak,
      input + sector * 512, output + sector * 512);
  }
  mbedtls_aes_free(&cipher);
  mbedtls_aes_free(&tweak_cipher);
  if (result || fwrite(output, 1, sizeof output, stdout) != sizeof output) {
    fputs("Synthetic header conversion failed.\n", stderr);
    return 1;
  }
  return 0;
}
