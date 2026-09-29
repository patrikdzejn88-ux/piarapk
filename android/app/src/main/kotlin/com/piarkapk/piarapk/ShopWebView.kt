package com.piarkapk.piarapk

import android.annotation.SuppressLint
import android.app.DownloadManager
import android.content.Context
import android.net.Uri
import android.os.Environment
import android.util.Log
import android.view.View
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import android.webkit.URLUtil
import io.flutter.plugin.platform.PlatformView
import io.flutter.plugin.platform.PlatformViewFactory
import io.flutter.plugin.common.StandardMessageCodec

/**
 * WebView магазина как PlatformView — встраивается ПРЯМО в окно приложения
 * (вкладка «Магазин»), без полноэкранной Activity.
 *
 * Логин/баланс (куки + DOM storage) сохраняются. Скачивание файлов (выдача
 * заказов) идёт через системный DownloadManager прямо в «Загрузки».
 */
object ShopWebViewHolder {
    var webView: WebView? = null
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

    private val webView: WebView

    init {
        webView = WebView(context)
        ShopWebViewHolder.webView = webView
        setupWebView()
    }

    @SuppressLint("SetJavaScriptEnabled")
    private fun setupWebView() {
        webView.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            databaseEnabled = true
            loadsImagesAutomatically = true
        }
        webView.webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(
                view: WebView,
                request: WebResourceRequest
            ): Boolean {
                val scheme = request.url.scheme ?: "https"
                // http/https открываем внутри; внешние схемы — системе
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
                onPageChanged?.invoke(url, view.canGoBack(), view.canGoForward())
            }
        }
        webView.webChromeClient = WebChromeClient()

        // скачивание выданных файлов → системный DownloadManager
        webView.setDownloadListener { url, _, contentDisposition, mimetype, _ ->
            try {
                val name = URLUtil.guessFileName(url, contentDisposition, mimetype)
                val request = DownloadManager.Request(Uri.parse(url))
                    .setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE_NOTIFY_COMPLETED)
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

        webView.loadUrl("https://dark.shopping/")
    }

    override fun getView(): View = webView

    override fun dispose() {
        if (ShopWebViewHolder.webView === webView) {
            ShopWebViewHolder.webView = null
        }
        webView.destroy()
    }
}
