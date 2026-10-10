// Prints the private key of one PKCS#12 entry as an unencrypted PKCS#8 PEM.
// Used by setup-certs.sh (the Elasticsearch image has a JDK but no openssl):
//   java PemKey.java <keystore.p12> <password> <alias> > node.key
import java.io.FileInputStream;
import java.security.Key;
import java.security.KeyStore;
import java.util.Base64;

public class PemKey {
    public static void main(String[] args) throws Exception {
        char[] password = args[1].toCharArray();
        KeyStore keyStore = KeyStore.getInstance("PKCS12");
        try (FileInputStream in = new FileInputStream(args[0])) {
            keyStore.load(in, password);
        }
        Key key = keyStore.getKey(args[2], password);
        String body = Base64.getMimeEncoder(64, "\n".getBytes()).encodeToString(key.getEncoded());
        System.out.println("-----BEGIN PRIVATE KEY-----\n" + body + "\n-----END PRIVATE KEY-----");
    }
}
