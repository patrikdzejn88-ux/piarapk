package com.piarkapk.piarapk

import android.annotation.SuppressLint
import android.os.Bundle
import android.view.View
import android.webkit.CookieManager
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.activity.OnBackPressedCallback
import androidx.fragment.app.FragmentActivity

/**
 * Встроенный браузер магазина dark.shopping (нативный WebView, без плагинов).
 *
 * ВАЖНО: базовый класс — ОБЫЧНЫЙ FragmentActivity (НЕ Flutter-класс!):
 * Flutter-активность создала бы второй FlutterEngine, который заново
 * исполняет main() — второй поллер событий ядра конкурировал бы с основным
 * приложением за результаты запросов.
 *
 * DOM-хранилище и куки включены — логин/баланс сайта сохраняются между
 * открытиями. Кнопка «назад» листает историю WebView, а не закрывает сразу.
 */
class WebViewActivity : FragmentActivity() {

    companion object {
        const val EXTRA_URL = "url"
    }

    private lateinit var webView: WebView

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val url = intent.getStringExtra(EXTRA_URL) ?: "https://dark.shopping/"

        webView = WebView(this)
        CookieManager.getInstance().setAcceptCookie(true)
        CookieManager.getInstance().setAcceptThirdPartyCookies(webView, true)
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
                // http/https открываем внутри; внешние схемы (tel:, mailto:) — системе
                val scheme = request.url.scheme ?: "https"
                if (scheme == "http" || scheme == "https") {
                    return false
                }
                return try {
                    startActivity(
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
                CookieManager.getInstance().flush()
                // показываем после первого рендера (без белой вспышки)
                view.visibility = View.VISIBLE
            }
        }
        webView.webChromeClient = WebChromeClient()

        setContentView(webView)
        webView.visibility = View.GONE
        webView.loadUrl(url)

        // «назад»: сначала история WebView, потом закрытие
        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() {
                if (webView.canGoBack()) {
                    webView.goBack()
                } else {
                    finish()
                }
            }
        })
    }
}
