package com.piarkapk.piarapk

import android.annotation.SuppressLint
import android.app.DownloadManager
import android.content.Context
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

/**
 * Один живой WebView на весь процесс: создаётся при старте Activity,
 * куки/DOM Storage пишутся на диск, вкладка не перезагружает сайт.
 */
object ShopWebViewHolder {
    const val SHOP_URL = "https://dark.shopping/"

    var webView: WebView? = null
        private set

    fun ensure(context: Context): WebView {
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
        cookies.setAcceptThirdPartyCookies(webView, true)

        webView.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            databaseEnabled = true
            loadsImagesAutomatically = true
            cacheMode = WebSettings.LOAD_DEFAULT
            mixedContentMode = WebSettings.MIXED_CONTENT_COMPATIBILITY_MODE
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
                val scheme = request.url.scheme ?: "https"
                if (scheme == "http" || scheme == "https") {
                    return false
                }
                return try {
                    view.context.startActivity(
                        android.content.Intent(
                            android.content.Intent.ACTION_VIEW,
                            request.url
                        )
                    )
                    true
                } catch (_: Exception) {
                    true
                }
            }

            override fun onPageFinished(view: WebView, url: String) {
                persistCookies()
                onPageChanged?.invoke(url, view.canGoBack(), view.canGoForward())
            }
        }
        webView.webChromeClient = WebChromeClient()

        webView.setDownloadListener { url, _, contentDisposition, mimetype, _ ->
            try {
                val name = URLUtil.guessFileName(url, contentDisposition, mimetype)
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
