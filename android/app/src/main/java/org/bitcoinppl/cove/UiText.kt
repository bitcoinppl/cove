package org.bitcoinppl.cove

import android.content.Context
import androidx.annotation.StringRes
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource

sealed interface UiText {
    data class Resource(
        @param:StringRes val id: Int,
        val args: List<Any> = emptyList(),
    ) : UiText

    data class Raw(val value: String) : UiText

    fun resolve(context: Context): String =
        when (this) {
            is Raw -> value
            is Resource -> {
                val resolvedArgs = args.map { arg -> resolveArg(context, arg) }.toTypedArray()
                if (resolvedArgs.isEmpty()) context.getString(id) else context.getString(id, *resolvedArgs)
            }
        }

    @Composable
    fun asString(): String =
        when (this) {
            is Raw -> value
            is Resource -> {
                val resolvedArgs = mutableListOf<Any>()
                for (arg in args) {
                    resolvedArgs.add(resolveArg(arg))
                }

                if (resolvedArgs.isEmpty()) stringResource(id) else stringResource(id, *resolvedArgs.toTypedArray())
            }
        }

    companion object {
        fun resource(
            @StringRes id: Int,
            vararg args: Any,
        ): UiText = Resource(id, args.toList())

        fun raw(value: String): UiText = Raw(value)

        private fun resolveArg(
            context: Context,
            arg: Any,
        ): Any =
            when (arg) {
                is UiText -> arg.resolve(context)
                else -> arg
            }

        @Composable
        private fun resolveArg(arg: Any): Any =
            when (arg) {
                is UiText -> arg.asString()
                else -> arg
            }
    }
}
