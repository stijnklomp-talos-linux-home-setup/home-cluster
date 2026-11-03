# Trusting the home-cluster certificate (CA)

> **Non-technical summary:** when you open a cluster website (ArgoCD, Tekton Dashboard…) your device checks "who signed this site's certificate?". The cluster has its own signer — the **root CA** — and your device doesn't recognise it yet, so you get "untrusted" / "your connection is not private" warnings. Installing the root CA once tells your device "this signer is OK", and the warnings go away. It's a one-time setup per device.

## 1. Get the CA file

Run this once on a computer that can talk to the cluster:

```sh
kubectl -n cert-manager get secret home-root-ca -o jsonpath='{.data.tls\.crt}' | base64 -d > home-ca.crt
```

You now have a file called `home-ca.crt`. Copy it to the device you want to trust (USB stick, AirDrop, email to yourself, etc.).

The current root CA (the 100-year `home-root-ca` reissued on 2026-09-12) has SHA-256 fingerprint `4E:D4:17:00:A0:1D:86:C9:21:3B:F2:FF:0E:75:CD:71:AF:84:54:7F:EC:84:62:4B:AE:AF:50:61:83:43:1E:9A`. If your exported file shows a different fingerprint, it is a stale CA — remove it (step 2) and export again.

## 2. Remove any OLD cluster CA first

If the device was set up before **2026-09-12**, it may still have the previous root CA stored. If you leave it behind, you'll end up with two "home-root-ca" entries and confusing warnings. **Remove old entries first, then install the new one.** Old CA fingerprint for reference: `6A:E0:CE:B3:48:BB:BD:DE:77:73:C5:EE:07:21:73:DC:0D:02:95:9A:35:87:CE:07:58:8E:66:79:C1:90:AE:E8`.

## 3. Install the new CA

### Windows
1. Double-click `home-ca.crt` → **Install Certificate** → select **Local Machine** (choose it if asked).
2. Select **Place all certificates in the following store** → **Browse** → **Trusted Root Certification Authorities** → OK.
3. **Finish**. Windows may ask you to confirm — click Yes.

### macOS
1. Double-click `home-ca.crt` → it opens in **Keychain Access**.
2. Choose the **System** keychain (you'll be asked for your password).
3. Right-click the new **home-root-ca** entry → **Get Info** → expand **Trust** → set **When using this certificate: Always Trust**.
4. Close — macOS asks for your password again. The warning is now gone.

### Linux (Ubuntu / Debian / Mint)
```sh
sudo cp home-ca.crt /usr/local/share/ca-certificates/home-root-ca.crt
sudo update-ca-certificates
```

### iPhone / iPad
1. Send `home-ca.crt` to the device (AirDrop, email, or Files).
2. Open the file → **Install Profile** → enter your passcode → **Install**.
3. Go to **Settings → General → About → Certificate Trust Settings** and switch the new certificate to **full trust**.

### Android
1. Download `home-ca.crt` (e.g. to **Downloads**).
2. Go to **Settings → Security → More security → Encryption & credentials → Install a certificate → CA certificate**.
3. Pick the file from Downloads and confirm.

### Firefox (separate! Firefox has its own certificate store)
1. Firefox → **Settings → Privacy & Security** (scroll to the bottom) → **Certificates → View Certificates**.
2. Tab **Authorities** → **Import** → select `home-ca.crt` → tick **Trust this CA to identify websites** → OK.

## 4. Verify

Open any cluster site, e.g. `https://argocd.<lb-ip>.sslip.io` (replace `<lb-ip>` with your ingress IP). If you no longer see a certificate warning, it worked.

Still warning? You likely have an old or duplicate entry — go back to step 2 and remove **all** "home-root-ca" entries, then reinstall the new file.

> This page describes the *cluster's own* root CA used for LAN websites. The public internet (GitHub, AWS…) uses well-known CAs and needs nothing from you.

## Go clients (Pinniped CLI, client-go) after the Istio cutover

Browsers, `curl` and OpenSSL compare certificate names canonically. Go's
`crypto/x509` compares the raw issuer/subject encoding, so a leaf whose issuer
was encoded as `PrintableString` does not chain to a CA whose subject is
`UTF8String`, even with the same key and name.

The 100-year `home-root-ca` reissue changed that encoding. On 2026-11-01
(ticket #12) the edge certificate (`internal-tls`) and the Pinniped supervisor
leaf (`supervisor-public-tls`) were reissued against the current root; the
impersonation-proxy chain was already consistent. Consequences:

- If a Go client fails with `x509: certificate signed by unknown authority`,
  point it at the **current** `home-root-ca` bundle (or regenerate the Pinniped
  kubeconfig). The old 90-day root certificate (`C0:3A:…`) is stale.
- The Pinniped kubeconfig embeds the **impersonation-proxy** CA, not the home
  root. Renewing the supervisor or edge leaf does not require a kubeconfig
  change; the JWTAuthenticator is re-rendered with the current root instead.
- Do not regenerate the root CA to fix a client. Reissue the leaf.