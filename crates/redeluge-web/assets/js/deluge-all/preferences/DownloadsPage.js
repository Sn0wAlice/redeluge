/**
 * Deluge.preferences.DownloadsPage.js
 *
 * Copyright (c) Damien Churchill 2009-2010 <damoxc@gmail.com>
 *
 * This file is part of Deluge and is licensed under GNU General Public License 3.0, or later, with
 * the additional special exception to link portions of this program with the OpenSSL library.
 * See LICENSE for more details.
 */
Ext.namespace('Deluge.preferences');

/**
 * @class Deluge.preferences.Downloads
 * @extends Ext.form.FormPanel
 */
Deluge.preferences.Downloads = Ext.extend(Ext.FormPanel, {
    constructor: function (config) {
        config = Ext.apply(
            {
                border: false,
                title: _('Downloads'),
                header: false,
                layout: 'form',
                autoHeight: true,
                width: 320,
            },
            config
        );
        Deluge.preferences.Downloads.superclass.constructor.call(this, config);
    },

    initComponent: function () {
        Deluge.preferences.Downloads.superclass.initComponent.call(this);

        var optMan = deluge.preferences.getOptionsManager();
        var fieldset = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('Folders'),
            labelWidth: 150,
            defaultType: 'togglefield',
            autoHeight: true,
            labelAlign: 'top',
            width: 300,
            style: 'margin-bottom: 5px; padding-bottom: 5px;',
        });

        optMan.bind(
            'download_location',
            fieldset.add({
                xtype: 'textfield',
                name: 'download_location',
                fieldLabel: _('Download to:'),
                labelSeparator: '',
                width: 280,
            })
        );

        var field = fieldset.add({
            name: 'move_completed_path',
            fieldLabel: _('Move completed to:'),
            labelSeparator: '',
            width: 280,
        });
        optMan.bind('move_completed', field.toggle);
        optMan.bind('move_completed_path', field.input);

        field = fieldset.add({
            name: 'torrentfiles_location',
            fieldLabel: _('Copy of .torrent files to:'),
            labelSeparator: '',
            width: 280,
        });
        optMan.bind('copy_torrent_file', field.toggle);
        optMan.bind('torrentfiles_location', field.input);

        fieldset = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('Options'),
            autoHeight: true,
            labelWidth: 1,
            defaultType: 'checkbox',
            style: 'margin-bottom: 0; padding-bottom: 0;',
            width: 280,
        });
        optMan.bind(
            'prioritize_first_last_pieces',
            fieldset.add({
                name: 'prioritize_first_last_pieces',
                labelSeparator: '',
                boxLabel: _('Prioritize first and last pieces of torrent'),
            })
        );
        optMan.bind(
            'sequential_download',
            fieldset.add({
                name: 'sequential_download',
                labelSeparator: '',
                boxLabel: _('Sequential download'),
            })
        );
        optMan.bind(
            'add_paused',
            fieldset.add({
                name: 'add_paused',
                labelSeparator: '',
                boxLabel: _('Add torrents in Paused state'),
            })
        );
        optMan.bind(
            'pre_allocate_storage',
            fieldset.add({
                name: 'pre_allocate_storage',
                labelSeparator: '',
                boxLabel: _('Pre-allocate disk space'),
            })
        );

        // The disk-space rule. Its own fieldset with its own load and apply,
        // because it is one dictionary under `disk_space` rather than flat
        // keys, and the options manager above only binds flat ones.
        fieldset = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('Free Space'),
            autoHeight: true,
            labelWidth: 200,
            style: 'padding-top: 5px; margin-bottom: 0px;',
            width: 300,
        });
        fieldset.add({
            xtype: 'label',
            text: _(
                'A disk that fills up puts every download into Error, one after another, and each one has to be restarted by hand. This stops them first and starts them again when there is room.'
            ),
            style: 'display: block; margin-bottom: 6px; color: #666;',
        });

        this.space = {};
        this.space.enabled = fieldset.add({
            xtype: 'checkbox',
            hideLabel: true,
            boxLabel: _('Pause downloads when the disk is nearly full'),
            handler: this.onSpaceToggled,
            scope: this,
        });
        this.space.min_free = fieldset.add(
            this.spaceSpinner(_('Pause below (GiB):'))
        );
        this.space.resume_free = fieldset.add(
            this.spaceSpinner(_('Resume above (GiB):'))
        );

        this.on('show', this.onPageShow, this);
    },

    /**
     * Both thresholds are the same field, in the unit people think in.
     */
    spaceSpinner: function (caption) {
        return {
            xtype: 'spinnerfield',
            fieldLabel: caption,
            labelSeparator: '',
            width: 80,
            decimalPrecision: 1,
            incrementValue: 0.5,
            minValue: 0,
            maxValue: 1024,
        };
    },

    onPageShow: function () {
        if (this.spaceLoaded) return;
        this.spaceLoaded = true;
        deluge.client.core.get_config_value('disk_space', {
            success: function (settings) {
                settings = settings || {};
                this.space.enabled.setValue(settings['enabled'] !== false);
                // Stored in bytes, shown in gibibytes, which is the unit the
                // thresholds are actually chosen in.
                this.space.min_free.setValue(
                    Deluge.preferences.Downloads.toGiB(settings['min_free'], 1)
                );
                this.space.resume_free.setValue(
                    Deluge.preferences.Downloads.toGiB(settings['resume_free'], 2)
                );
                this.onSpaceToggled();
            },
            failure: function () {
                this.onSpaceToggled();
            },
            scope: this,
        });
    },

    /**
     * The thresholds mean nothing while the rule is off, so they follow it.
     */
    onSpaceToggled: function () {
        var on = this.space.enabled.getValue() === true;
        this.space.min_free.setDisabled(!on);
        this.space.resume_free.setDisabled(!on);
    },

    onApply: function () {
        // Nothing read yet means nothing of this rule's to write: Preferences
        // applies every page on OK, and an unread page holds its defaults.
        if (!this.spaceLoaded) return;

        var gib = 1024 * 1024 * 1024;
        deluge.client.core.set_config({
            disk_space: {
                enabled: this.space.enabled.getValue() === true,
                min_free: Math.round(
                    Deluge.number(this.space.min_free.getValue(), 1) * gib
                ),
                resume_free: Math.round(
                    Deluge.number(this.space.resume_free.getValue(), 2) * gib
                ),
            },
        });
    },
});

/**
 * Bytes as gibibytes, to one decimal, falling back to a default the daemon
 * would have used anyway.
 */
Deluge.preferences.Downloads.toGiB = function (bytes, fallback) {
    var value = Number(bytes);
    if (!isFinite(value) || value < 0) return fallback;
    return Math.round((value / (1024 * 1024 * 1024)) * 10) / 10;
};
