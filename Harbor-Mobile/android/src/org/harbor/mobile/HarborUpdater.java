package org.harbor.mobile;

import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Environment;

import androidx.core.content.FileProvider;

import java.io.BufferedReader;
import java.io.File;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.io.RandomAccessFile;
import java.net.HttpURLConnection;
import java.net.URL;
import java.security.MessageDigest;
import java.util.Locale;

import org.json.JSONArray;
import org.json.JSONObject;

/**
 * Mandatory in-app updater for Harbor Mobile. Same policy as the desktop:
 * updates are mandatory once DISCOVERED (available/ready) — the shell
 * blocks on them with no skip path — while a mere check failure never
 * blocks: offline stays usable and the host retries on its own cadence.
 *
 * No JNI upcalls: the worker threads below only mutate an atomic state
 * string the C++ facade polls (HarborAndroid.updateState). All traffic is
 * plain HTTPS to the GitHub release channel, and the APK is SHA-256
 * verified against its sibling .sha256 asset before the system installer
 * is ever invoked (installing itself still goes through the platform UI).
 */
public final class HarborUpdater {
    private static final String OWNER = "Joshua-lvt";
    private static final String REPO = "Harbor";
    private static final String ASSET_APK = "harbor-android-arm64-debug.apk";
    private static final String USER_AGENT = "Harbor-Mobile-Updater";

    // idle | checking | available | downloading | ready | error
    private static volatile String state =
            "{\"status\":\"idle\",\"version\":\"\",\"progress\":0,\"error\":\"\",\"url\":\"\",\"sha\":\"\"}";
    private static volatile boolean workerRunning = false;

    private HarborUpdater() {}

    /** Last known state as JSON for the C++ poller. Never blocks. */
    public static String updateState(Context context) {
        return state;
    }

    private static void setState(String status, String version, double progress,
                                 String error, String url, String sha) {
        try {
            JSONObject json = new JSONObject();
            json.put("status", status);
            json.put("version", version == null ? "" : version);
            json.put("progress", progress);
            json.put("error", error == null ? "" : error);
            json.put("url", url == null ? "" : url);
            json.put("sha", sha == null ? "" : sha);
            state = json.toString();
        } catch (Exception ignored) {
        }
    }

    private static String currentVersion(Context context) {
        try {
            PackageManager manager = context.getPackageManager();
            PackageInfo info = manager.getPackageInfo(context.getPackageName(), 0);
            return info.versionName == null ? "0.0.0" : info.versionName;
        } catch (Exception e) {
            return "0.0.0";
        }
    }

    /** -1/0/+1 over dotted versions; leading "v" tolerated. */
    static int compareVersions(String a, String b) {
        String cleanA = a == null ? "" : a.trim();
        String cleanB = b == null ? "" : b.trim();
        if (cleanA.startsWith("v") || cleanA.startsWith("V"))
            cleanA = cleanA.substring(1);
        if (cleanB.startsWith("v") || cleanB.startsWith("V"))
            cleanB = cleanB.substring(1);
        String tailA = "";
        String tailB = "";
        int dash = cleanA.indexOf('-');
        if (dash >= 0) {
            tailA = cleanA.substring(dash + 1);
            cleanA = cleanA.substring(0, dash);
        }
        dash = cleanB.indexOf('-');
        if (dash >= 0) {
            tailB = cleanB.substring(dash + 1);
            cleanB = cleanB.substring(0, dash);
        }
        String[] partsA = cleanA.split("\\.");
        String[] partsB = cleanB.split("\\.");
        int count = Math.max(partsA.length, partsB.length);
        for (int i = 0; i < count; i++) {
            int left = i < partsA.length ? parsePart(partsA[i]) : 0;
            int right = i < partsB.length ? parsePart(partsB[i]) : 0;
            if (left != right)
                return left < right ? -1 : 1;
        }
        if (tailA.equals(tailB))
            return 0;
        if (tailA.isEmpty())
            return 1;
        if (tailB.isEmpty())
            return -1;
        return tailA.compareTo(tailB) < 0 ? -1 : 1;
    }

    private static int parsePart(String piece) {
        try {
            return Integer.parseInt(piece);
        } catch (NumberFormatException e) {
            return 0;
        }
    }

    private static String readAll(InputStream in) throws Exception {
        BufferedReader reader = new BufferedReader(new InputStreamReader(in, "UTF-8"));
        StringBuilder out = new StringBuilder();
        char[] buffer = new char[4096];
        int read;
        while ((read = reader.read(buffer)) >= 0)
            out.append(buffer, 0, read);
        return out.toString();
    }

    /** Start one release-channel check (no-op while a worker runs). */
    public static void checkForUpdates(final Context context) {
        final Context app = context.getApplicationContext();
        synchronized (HarborUpdater.class) {
            if (workerRunning)
                return;
            workerRunning = true;
        }
        setState("checking", "", 0, "", "", "");
        new Thread(new Runnable() {
            @Override
            public void run() {
                try {
                    URL url = new URL("https://api.github.com/repos/" + OWNER + "/" + REPO
                            + "/releases/latest");
                    HttpURLConnection connection = (HttpURLConnection) url.openConnection();
                    connection.setRequestProperty("Accept", "application/vnd.github+json");
                    connection.setRequestProperty("User-Agent", USER_AGENT);
                    connection.setConnectTimeout(15000);
                    connection.setReadTimeout(15000);
                    if (connection.getResponseCode() != 200) {
                        setState("error", "", 0, "update.error.network", "", "");
                        return;
                    }
                    JSONObject release = new JSONObject(readAll(connection.getInputStream()));
                    if (release.optBoolean("prerelease", false)) {
                        setState("idle", "", 0, "", "", "");
                        return;
                    }
                    String tag = release.optString("tag_name", "");
                    if (tag.isEmpty() || compareVersions(tag, currentVersion(app)) <= 0) {
                        setState("idle", "", 0, "", "", "");
                        return;
                    }
                    String download = "";
                    String sha = "";
                    JSONArray assets = release.optJSONArray("assets");
                    if (assets != null) {
                        for (int i = 0; i < assets.length(); i++) {
                            JSONObject asset = assets.optJSONObject(i);
                            if (asset == null)
                                continue;
                            String name = asset.optString("name", "");
                            if (name.equals(ASSET_APK))
                                download = asset.optString("browser_download_url", "");
                            else if (name.equals(ASSET_APK + ".sha256"))
                                sha = asset.optString("browser_download_url", "");
                        }
                    }
                    if (download.isEmpty()) {
                        setState("error", "", 0, "update.error.noArtifact", "", "");
                        return;
                    }
                    String version = tag.startsWith("v") ? tag.substring(1) : tag;
                    setState("available", version, 0, "", download, sha);
                } catch (Exception e) {
                    setState("error", "", 0, "update.error.network", "", "");
                } finally {
                    synchronized (HarborUpdater.class) {
                        workerRunning = false;
                    }
                }
            }
        }).start();
    }

    /** Download the discovered package with progress (no-op while busy). */
    public static void downloadUpdate(final Context context, final String url, final String shaUrl) {
        final Context app = context.getApplicationContext();
        synchronized (HarborUpdater.class) {
            if (workerRunning)
                return;
            workerRunning = true;
        }
        new Thread(new Runnable() {
            @Override
            public void run() {
                File target = new File(app.getCacheDir(), "harbor-update.apk");
                try {
                    String version = versionOf(state);
                    setState("downloading", version, 0, "", url, shaUrl);
                    HttpURLConnection connection = (HttpURLConnection) new URL(url).openConnection();
                    connection.setRequestProperty("User-Agent", USER_AGENT);
                    connection.setConnectTimeout(15000);
                    connection.setReadTimeout(30000);
                    if (connection.getResponseCode() != 200) {
                        setState("error", "", 0, "update.error.network", "", "");
                        return;
                    }
                    int total = connection.getContentLength();
                    InputStream in = connection.getInputStream();
                    RandomAccessFile out = new RandomAccessFile(target, "rw");
                    out.setLength(0);
                    byte[] buffer = new byte[65536];
                    long received = 0;
                    int read;
                    while ((read = in.read(buffer)) >= 0) {
                        out.write(buffer, 0, read);
                        received += read;
                        if (total > 0)
                            setState("downloading", version,
                                    Math.max(0.0, Math.min(1.0, received / (double) total)),
                                    "", url, shaUrl);
                    }
                    out.close();
                    in.close();
                    if (shaUrl == null || shaUrl.isEmpty()) {
                        target.delete();
                        setState("error", "", 0, "update.error.noChecksum", "", "");
                        return;
                    }
                    String expected = fetchChecksum(shaUrl);
                    if (expected.isEmpty() || !verifyChecksum(target, expected)) {
                        target.delete();
                        setState("error", "", 0, "update.error.checksum", "", "");
                        return;
                    }
                    setState("ready", version, 1, "", url, shaUrl);
                } catch (Exception e) {
                    target.delete();
                    setState("error", "", 0, "update.error.network", "", "");
                } finally {
                    synchronized (HarborUpdater.class) {
                        workerRunning = false;
                    }
                }
            }
        }).start();
    }

    private static String versionOf(String stateJson) {
        try {
            return new JSONObject(stateJson).optString("version", "");
        } catch (Exception e) {
            return "";
        }
    }

    private static String fetchChecksum(String shaUrl) {
        try {
            HttpURLConnection connection = (HttpURLConnection) new URL(shaUrl).openConnection();
            connection.setRequestProperty("User-Agent", USER_AGENT);
            connection.setConnectTimeout(15000);
            connection.setReadTimeout(15000);
            if (connection.getResponseCode() != 200)
                return "";
            String body = readAll(connection.getInputStream()).trim();
            int space = body.indexOf(' ');
            return space > 0 ? body.substring(0, space) : body;
        } catch (Exception e) {
            return "";
        }
    }

    private static boolean verifyChecksum(File file, String expectedHex) {
        try {
            MessageDigest digest = MessageDigest.getInstance("SHA-256");
            InputStream in = new java.io.FileInputStream(file);
            byte[] buffer = new byte[65536];
            int read;
            while ((read = in.read(buffer)) >= 0)
                digest.update(buffer, 0, read);
            in.close();
            byte[] hash = digest.digest();
            StringBuilder hex = new StringBuilder();
            for (byte b : hash)
                hex.append(String.format(Locale.US, "%02x", b));
            return hex.toString().equalsIgnoreCase(expectedHex.trim());
        } catch (Exception e) {
            return false;
        }
    }

    /** Hand the verified package to the platform installer UI. */
    public static void installUpdate(Context context) {
        try {
            File apk = new File(context.getCacheDir(), "harbor-update.apk");
            if (!apk.exists())
                return;
            android.net.Uri uri = FileProvider.getUriForFile(
                    context, context.getPackageName() + ".fileprovider", apk);
            Intent intent = new Intent(Intent.ACTION_INSTALL_PACKAGE);
            intent.setDataAndType(uri, "application/vnd.android.package-archive");
            intent.setFlags(Intent.FLAG_ACTIVITY_NEW_TASK
                    | Intent.FLAG_GRANT_READ_URI_PERMISSION);
            context.startActivity(intent);
        } catch (Exception ignored) {
        }
    }
}
