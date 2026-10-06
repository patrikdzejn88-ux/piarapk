package com.piarkapk.piarapk

import android.annotation.SuppressLint
import android.app.Activity
import android.app.AlertDialog
import android.app.DownloadManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Environment
import android.util.Log
import android.view.View
import android.view.ViewGroup
import android.webkit.CookieManager
import android.webkit.URLUtil
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import io.flutter.plugin.common.StandardMessageCodec
import io.flutter.plugin.platform.PlatformView
import io.flutter.plugin.platform.PlatformViewFactory
import java.lang.ref.WeakReference

/** Схемы, которые разрешено открывать во внешнем приложении. */
private val EXTERNAL_SCHEMES = setOf("tel", "mailto")

/**
 * Открывает внешнюю ссылку только из белого списка схем (tel/mailto).
 * Произвольные схемы (intent:, market:, file: и т.п.) игнорируются.
 */
internal fun openExternalScheme(context: Context, uri: Uri): Boolean {
    val scheme = uri.scheme?.lowercase() ?: return false
    if (scheme !in EXTERNAL_SCHEMES) return false
    return try {
        val intent = Intent(Intent.ACTION_VIEW, uri)
        if (context !is Activity) {
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        }
        context.startActivity(intent)
        true
    } catch (_: Exception) {
        true
    }
}

/**
 * Один живой WebView на весь процесс: создаётся при старте Activity,
 * куки/DOM Storage пишутся на диск, вкладка не перезагружает сайт.
 */
object ShopWebViewHolder {
    const val SHOP_URL = "https://dark.shopping/"

    var webView: WebView? = null
        private set

    /** Хост-Activity только для показа диалогов (слабая ссылка — без утечки). */
    private var hostActivity: WeakReference<Activity>? = null

    fun ensure(context: Context): WebView {
        if (context is Activity) {
            hostActivity = WeakReference(context)
        }
        webView?.let { return it }
        val wv = WebView(context.applicationContext)
        webView = wv
        setup(wv)
        wv.loadUrl(SHOP_URL)
        return wv
    }

    fun persistCookies() {
        try {
            CookieManager.getInstance().flush()
        } catch (e: Exception) {
            Log.w("piarapk", "cookie flush failed", e)
        }
    }

    @SuppressLint("SetJavaScriptEnabled")
    private fun setup(webView: WebView) {
        val cookies = CookieManager.getInstance()
        cookies.setAcceptCookie(true)
        // Сторонние куки магазину не нужны — выключаем.
        cookies.setAcceptThirdPartyCookies(webView, false)

        webView.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            databaseEnabled = true
            loadsImagesAutomatically = true
            cacheMode = WebSettings.LOAD_DEFAULT
            mixedContentMode = WebSettings.MIXED_CONTENT_NEVER_ALLOW
            allowFileAccess = false
            allowContentAccess = false
            useWideViewPort = true
            loadWithOverviewMode = true
            setSupportZoom(false)
            mediaPlaybackRequiresUserGesture = true
        }
        webView.webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(
                view: WebView,
                request: WebResourceRequest
            ): Boolean {
                val scheme = request.url.scheme?.lowercase() ?: return false
                if (scheme == "http" || scheme == "https") {
                    return false
                }
                openExternalScheme(view.context, request.url)
                return true
            }

            override fun onPageFinished(view: WebView, url: String) {
                persistCookies()
                onPageChanged?.invoke(url, view.canGoBack(), view.canGoForward())
            }
        }
        webView.webChromeClient = WebChromeClient()

        webView.setDownloadListener { url, _, contentDisposition, mimetype, _ ->
            val activity = hostActivity?.get()
            if (activity == null) {
                Log.w("piarapk", "shop download ignored: host activity unavailable")
                return@setDownloadListener
            }
            try {
                val name = URLUtil.guessFileName(url, contentDisposition, mimetype)
                AlertDialog.Builder(activity)
                    .setTitle("Скачать файл?")
                    .setMessage(name)
                    .setPositiveButton("Скачать") { _, _ ->
                        enqueueDownload(webView, url, name, mimetype)
                    }
                    .setNegativeButton("Отмена", null)
                    .show()
            } catch (e: Exception) {
                Log.e("piarapk", "shop download confirm failed", e)
            }
        }
    }

    private fun enqueueDownload(webView: WebView, url: String, name: String, mimetype: String?) {
        try {
            val request = DownloadManager.Request(Uri.parse(url))
                .setNotificationVisibility(
                    DownloadManager.Request.VISIBILITY_VISIBLE_NOTIFY_COMPLETED
                )
                .setDestinationInExternalPublicDir(
                    Environment.DIRECTORY_DOWNLOADS, name
                )
            if (mimetype != null) request.setMimeType(mimetype)
            (webView.context.getSystemService(Context.DOWNLOAD_SERVICE)
                as DownloadManager).enqueue(request)
        } catch (e: Exception) {
            Log.e("piarapk", "shop download failed", e)
        }
    }
}

/** Сообщает Dart'у смену URL и состояние навигации. */
private var onPageChanged: ((url: String, canBack: Boolean, canFwd: Boolean) -> Unit)? = null

class ShopWebViewFactory(
    private val notifyPage: (String, Boolean, Boolean) -> Unit
) : PlatformViewFactory(StandardMessageCodec.INSTANCE) {

    override fun create(context: Context, viewId: Int, args: Any?): PlatformView {
        onPageChanged = notifyPage
        return ShopWebView(context)
    }
}

class ShopWebView(context: Context) : PlatformView {

    private val webView: WebView = ShopWebViewHolder.ensure(context)

    init {
        (webView.parent as? ViewGroup)?.removeView(webView)
    }

    override fun getView(): View = webView

    override fun dispose() {
        ShopWebViewHolder.persistCookies()
        (webView.parent as? ViewGroup)?.removeView(webView)
    }
}