/**
 * Deluge.preferences.Blocklist.js
 *
 * Copyright (c) 2026 the redeluge contributors
 *
 * This file is part of redeluge and is licensed under GNU General Public License 3.0, or later,
 * with the additional special exception to link portions of this program with the OpenSSL library.
 * See LICENSE for more details.
 */
Ext.namespace('Deluge.preferences');

/**
 * @class Deluge.preferences.Blocklist
 * @extends Ext.form.FormPanel
 *
 * The peer block list. Was the Blocklist plugin; now the `blocklist` key of
 * core.conf, so the page reads and writes that one dictionary.
 *
 * Two of its fields are written by the daemon rather than by anyone here, and
 * are shown read-only: when the list was last fetched, and how many ranges it
 * produced.
 */
Deluge.preferences.Blocklist = Ext.extend(Ext.form.FormPanel, {
    border: false,
    title: _('Block List'),
    header: false,
    layout: 'form',
    autoScroll: true,

    initComponent: function () {
        Deluge.preferences.Blocklist.superclass.initComponent.call(this);

        var settings = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('Settings'),
            autoHeight: true,
            labelWidth: 1,
            style: 'padding-top: 5px; margin-bottom: 0px;',
        });

        this.enabled = settings.add({
            xtype: 'checkbox',
            fieldLabel: '',
            labelSeparator: '',
            boxLabel: _('Block peers on a downloaded list'),
            name: 'blocklist_enabled',
        });

        var source = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('List'),
            autoHeight: true,
            labelWidth: 150,
            style: 'padding-top: 5px; margin-bottom: 0px;',
        });

        this.url = source.add({
            xtype: 'textfield',
            fieldLabel: _('URL:'),
            labelSeparator: '',
            name: 'blocklist_url',
            width: 320,
        });
        source.add({
            xtype: 'label',
            text: _('Plain, gzipped or zipped. PeerGuardian and eMule formats are read.'),
            style: 'display: block; margin: 4px 0 8px 155px; opacity: 0.72;',
        });

        this.check_after_days = source.add({
            xtype: 'spinnerfield',
            fieldLabel: _('Check every (days):'),
            labelSeparator: '',
            name: 'blocklist_check_after_days',
            width: 80,
            decimalPrecision: 0,
            minValue: 0,
        });
        source.add({
            xtype: 'label',
            text: _('Zero never fetches it again, which pins a list you provided yourself.'),
            style: 'display: block; margin: 4px 0 8px 155px; opacity: 0.72;',
        });

        this.timeout = source.add({
            xtype: 'spinnerfield',
            fieldLabel: _('Timeout (seconds):'),
            labelSeparator: '',
            name: 'blocklist_timeout',
            width: 80,
            decimalPrecision: 0,
            minValue: 1,
        });
        this.try_times = source.add({
            xtype: 'spinnerfield',
            fieldLabel: _('Attempts:'),
            labelSeparator: '',
            name: 'blocklist_try_times',
            width: 80,
            decimalPrecision: 0,
            minValue: 1,
        });

        var allowed = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('Never Blocked'),
            autoHeight: true,
            labelWidth: 1,
            style: 'padding-top: 5px; margin-bottom: 0px;',
        });
        this.whitelisted = allowed.add({
            xtype: 'textarea',
            fieldLabel: '',
            labelSeparator: '',
            name: 'blocklist_whitelisted',
            width: 320,
            height: 90,
        });
        allowed.add({
            xtype: 'label',
            text: _('One address or range per line, such as 10.0.0.1 or 10.0.0.0 - 10.255.255.255.'),
            style: 'display: block; margin: 4px 0 8px 0; opacity: 0.72;',
        });

        var status = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('Status'),
            autoHeight: true,
            labelWidth: 150,
            style: 'padding-top: 5px; margin-bottom: 0px;',
        });
        this.status = status.add({
            xtype: 'label',
            text: _('Not loaded.'),
            style: 'display: block; opacity: 0.72;',
        });

        // There is no RPC method for "fetch now", and there should not be: the
        // daemon answers Deluge's API and nothing else. Clearing the timestamp
        // is enough. The maintenance loop looks once a minute and treats a
        // list it has never fetched as stale, so this brings the download
        // forward without inventing a method for it.
        status.add({
            xtype: 'button',
            text: _('Fetch Now'),
            width: 100,
            style: 'margin-top: 6px',
            handler: this.onFetchNow,
            scope: this,
        });

        this.on('show', this.onPageShow, this);
    },

    // The card layout fires `show` whenever the window opens on this page and
    // again on every switch back to it. Fetching each time would be a call per
    // click; once is enough, and the Apply button writes rather than reads.
    onPageShow: function () {
        if (this.loaded) return;
        this.loaded = true;
        deluge.client.core.get_config({
            success: this.onGotConfig,
            scope: this,
        });
    },

    onGotConfig: function (config) {
        var settings = config['blocklist'] || {};
        // Kept whole. `core.set_config` replaces the key rather than merging
        // into it, so a page that sends only its own fields drops the two the
        // daemon writes, `last_update` and `list_size`. That made every Apply
        // look like a list that had never been fetched, and the next check
        // downloaded it again.
        this.settings = settings;
        this.enabled.setValue(settings['enabled'] === true);
        this.url.setValue(Ext.value(settings['url'], ''));
        this.check_after_days.setValue(Ext.value(settings['check_after_days'], 4));
        this.timeout.setValue(Ext.value(settings['timeout'], 180));
        this.try_times.setValue(Ext.value(settings['try_times'], 3));
        this.whitelisted.setValue((settings['whitelisted'] || []).join('\n'));

        var last = Number(settings['last_update'] || 0);
        var size = Number(settings['list_size'] || 0);
        if (last > 0) {
            this.status.setText(
                String.format(
                    _('{0} ranges, last fetched {1}.'),
                    size,
                    new Date(last * 1000).toLocaleString()
                )
            );
        } else {
            this.status.setText(_('Never fetched.'));
        }
    },

    onApply: function () {
        // Preferences applies every page, including ones nobody opened.
        // Until this page has read the stored settings its fields hold
        // defaults and its grid is empty, and writing that back would replace
        // what is configured with nothing.
        if (!this.loaded) return;

        var lines = (this.whitelisted.getValue() || '')
            .split('\n')
            .map(function (line) {
                return line.trim();
            })
            .filter(function (line) {
                return line.length > 0;
            });

        this.write({
            enabled: this.enabled.getValue(),
            url: this.url.getValue(),
            check_after_days: Deluge.number(
                this.check_after_days.getValue(),
                4
            ),
            timeout: Deluge.number(this.timeout.getValue(), 180),
            try_times: Deluge.number(this.try_times.getValue(), 3),
            whitelisted: lines,
        });
    },

    /**
     * Writes the whole dictionary back, the daemon's own keys included.
     */
    write: function (changes) {
        var settings = Ext.apply({}, this.settings || {});
        Ext.apply(settings, changes);
        deluge.client.core.set_config({ blocklist: settings });
        this.settings = settings;
    },

    onFetchNow: function () {
        if (!this.loaded) return;
        this.onApply();
        this.write({ last_update: 0 });
        this.status.setText(_('Fetching within the minute.'));
    },

    onOk: function () {
        this.onApply();
    },
});
